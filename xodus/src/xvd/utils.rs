use std::fmt::Debug;
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use reqwest::{
    IntoUrl, StatusCode, Url, blocking,
    header::{CONTENT_RANGE, RANGE},
};

use ntfs::{Ntfs, NtfsFile, NtfsReadSeek};
use rsa::sha2::{self, Digest};
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncSeekExt},
};
use zerocopy::{IntoBytes, transmute};

use crate::models::xvd::{
    PAGE_SIZE, XvdSegmentMetadataHeader, XvdSegmentMetadataSegment, XvdUserDataHeader,
    XvdUserDataPackageFileEntry, XvdUserDataPackageFilesHeader,
};
use crate::xvd::crypt::{SectionReader, transform_page_xts};
use crate::xvd::math::{
    bytes_to_pages, calculate_hash_block_num_for_block_num, offset_to_page_number,
};
use crate::{
    models::xvd::{XvcInfo, XvcRegionHeader, XvcRegionSpecifier, XvdHeader, XvdUpdateSegment},
    xvd::math::page_number_to_offset,
};

const DEFAULT_HTTP_READ_AHEAD_BYTES: usize = 4 * 1024 * 1024;
const SMALL_FORWARD_SEEK_LIMIT: usize = 4 * 1024 * 1024;
const PREFIX_CACHE_LIMIT_BYTES: u64 = 50 * 1024 * 1024;

fn seek_target(current: u64, len: u64, pos: SeekFrom) -> io::Result<u64> {
    let new_offset = match pos {
        SeekFrom::Start(n) => n,
        SeekFrom::Current(delta) => if delta >= 0 {
            current.checked_add(delta as u64)
        } else {
            current.checked_sub(delta.unsigned_abs())
        }
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid relative seek"))?,
        SeekFrom::End(delta) => if delta >= 0 {
            len.checked_add(delta as u64)
        } else {
            len.checked_sub(delta.unsigned_abs())
        }
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid end-relative seek"))?,
    };

    if new_offset > len {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "seek past virtual device end",
        ));
    }

    Ok(new_offset)
}

fn reqwest_err(context: &str, err: reqwest::Error) -> io::Error {
    Error::other(format!("{context}: {err}"))
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    let (_, total) = value.split_once('/')?;
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

fn http_trace_enabled() -> bool {
    std::env::var_os("XODUS_HTTP_TRACE").is_some()
}

#[derive(Debug)]
struct ReadAheadBuffer {
    start: u64,
    data: Vec<u8>,
    capacity: usize,
}

impl ReadAheadBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            start: 0,
            data: Vec::new(),
            capacity,
        }
    }

    fn end(&self) -> u64 {
        self.start + self.data.len() as u64
    }

    fn contains(&self, offset: u64) -> bool {
        offset >= self.start && offset < self.end()
    }

    fn read_into(&self, offset: u64, buf: &mut [u8]) -> usize {
        if buf.is_empty() || !self.contains(offset) {
            return 0;
        }

        let start = (offset - self.start) as usize;
        let to_copy = (self.data.len() - start).min(buf.len());
        buf[..to_copy].copy_from_slice(&self.data[start..start + to_copy]);
        to_copy
    }

    fn append(&mut self, chunk_start: u64, chunk: &[u8]) {
        if self.capacity == 0 || chunk.is_empty() {
            return;
        }

        if self.data.is_empty() || self.end() != chunk_start {
            let keep = chunk.len().min(self.capacity);
            let skip = chunk.len() - keep;
            self.start = chunk_start + skip as u64;
            self.data.clear();
            self.data.extend_from_slice(&chunk[skip..]);
            return;
        }

        self.data.extend_from_slice(chunk);
        if self.data.len() > self.capacity {
            let overflow = self.data.len() - self.capacity;
            self.data.drain(..overflow);
            self.start += overflow as u64;
        }
    }
}

struct ActiveHttpRange {
    next_offset: u64,
    end_offset: u64,
    response: blocking::Response,
}

pub struct HttpFile {
    client: blocking::Client,
    url: Url,
    len: u64,
    pos: u64,
    read_ahead_bytes: usize,
    cache: ReadAheadBuffer,
    active: Option<ActiveHttpRange>,
}

impl HttpFile {
    pub fn open(url: impl IntoUrl) -> io::Result<Self> {
        Self::with_client_and_readahead(blocking::Client::new(), url, DEFAULT_HTTP_READ_AHEAD_BYTES)
    }

    pub fn with_client(client: blocking::Client, url: impl IntoUrl) -> io::Result<Self> {
        Self::with_client_and_readahead(client, url, DEFAULT_HTTP_READ_AHEAD_BYTES)
    }

    pub fn with_readahead(url: impl IntoUrl, read_ahead_bytes: usize) -> io::Result<Self> {
        Self::with_client_and_readahead(blocking::Client::new(), url, read_ahead_bytes)
    }

    pub fn with_client_and_readahead(
        client: blocking::Client,
        url: impl IntoUrl,
        read_ahead_bytes: usize,
    ) -> io::Result<Self> {
        let url = url
            .into_url()
            .map_err(|err| Error::new(ErrorKind::InvalidInput, err))?;
        let len = Self::discover_len(&client, &url)?;
        let read_ahead_bytes = read_ahead_bytes.max(1);

        Ok(Self {
            client,
            url,
            len,
            pos: 0,
            read_ahead_bytes,
            cache: ReadAheadBuffer::new(read_ahead_bytes),
            active: None,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn trace(&self, message: impl AsRef<str>) {
        if http_trace_enabled() {
            eprintln!("[httpfile] {}", message.as_ref());
        }
    }

    fn discover_len(client: &blocking::Client, url: &Url) -> io::Result<u64> {
        if let Ok(response) = client.head(url.clone()).send() {
            let response = response
                .error_for_status()
                .map_err(|err| reqwest_err("HTTP HEAD failed", err))?;
            if let Some(len) = response.content_length() {
                if len > 0 {
                    return Ok(len);
                }
            }
        }

        let response = client
            .get(url.clone())
            .header(RANGE, "bytes=0-0")
            .send()
            .map_err(|err| reqwest_err("HTTP range probe failed", err))?;
        let status = response.status();
        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            if let Some(total) = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range_total)
            {
                return Ok(total);
            }
        }
        let response = response
            .error_for_status()
            .map_err(|err| reqwest_err("HTTP range probe failed", err))?;

        if let Some(total) = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range_total)
        {
            return Ok(total);
        }

        if status == StatusCode::OK {
            if let Some(len) = response.content_length() {
                return Ok(len);
            }
        }

        Err(Error::new(
            ErrorKind::InvalidData,
            "unable to determine HTTP object length",
        ))
    }

    fn reopen_active_range(&mut self) -> io::Result<()> {
        if self.active.is_some() {
            self.trace(format!("dropping active range before reopen at pos={}", self.pos));
        }
        self.active = None;
        if self.pos >= self.len {
            return Ok(());
        }

        let range_end = self
            .pos
            .saturating_add(self.read_ahead_bytes as u64)
            .saturating_sub(1)
            .min(self.len - 1);
        let response = self
            .client
            .get(self.url.clone())
            .header(RANGE, format!("bytes={}-{}", self.pos, range_end))
            .send()
            .map_err(|err| reqwest_err("HTTP range request failed", err))?;
        let status = response.status();
        let response = response
            .error_for_status()
            .map_err(|err| reqwest_err("HTTP range request failed", err))?;

        let end_offset = match status {
            StatusCode::PARTIAL_CONTENT => {
                self.pos
                    + response
                        .content_length()
                        .unwrap_or(range_end - self.pos + 1)
            }
            StatusCode::OK if self.pos == 0 => self.len,
            StatusCode::OK => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "server ignored byte range request",
                ));
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("unexpected HTTP status for range request: {status}"),
                ));
            }
        };

        self.active = Some(ActiveHttpRange {
            next_offset: self.pos,
            end_offset,
            response,
        });
        self.trace(format!(
            "opened range status={} start={} end={} len={}",
            status, self.pos, end_offset, end_offset - self.pos
        ));
        Ok(())
    }

    fn skip_with_active_stream(&mut self, skip: usize) -> io::Result<bool> {
        if skip == 0 {
            return Ok(true);
        }

        let Some((available, next_offset, end_offset)) = self
            .active
            .as_ref()
            .map(|active| {
                (
                    active.end_offset.saturating_sub(active.next_offset) as usize,
                    active.next_offset,
                    active.end_offset,
                )
            })
        else {
            return Ok(false);
        };

        if skip > available {
            self.trace(format!(
                "cannot skip in active stream: skip={} available={} pos={}",
                skip, available, self.pos
            ));
            return Ok(false);
        }

        self.trace(format!(
            "skipping forward in active stream: skip={} pos={} available={} active_next={} active_end={}",
            skip, self.pos, available, next_offset, end_offset
        ));

        let active = self.active.as_mut().expect("active stream disappeared");
        let mut remaining = skip;
        let mut scratch = [0u8; 8192];

        while remaining > 0 {
            let chunk_len = remaining.min(scratch.len());
            let read = active.response.read(&mut scratch[..chunk_len])?;
            if read == 0 {
                self.active = None;
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "HTTP response ended while skipping forward in the active range",
                ));
            }

            let chunk_start = active.next_offset;
            active.next_offset += read as u64;
            self.cache.append(chunk_start, &scratch[..read]);
            remaining -= read;
        }

        if active.next_offset >= active.end_offset {
            self.active = None;
        }

        self.pos += skip as u64;
        Ok(true)
    }

    fn read_from_cache(&mut self, buf: &mut [u8]) -> usize {
        let read = self.cache.read_into(self.pos, buf);
        self.pos += read as u64;
        read
    }

    fn read_from_active(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(active) = self.active.as_mut() else {
            return Ok(0);
        };

        let available = active.end_offset.saturating_sub(active.next_offset) as usize;
        if available == 0 {
            self.active = None;
            return Ok(0);
        }

        let to_read = available.min(buf.len());
        let read = active.response.read(&mut buf[..to_read])?;
        if read == 0 {
            self.active = None;
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "HTTP response ended before the requested range was fully read",
            ));
        }

        let chunk_start = active.next_offset;
        active.next_offset += read as u64;
        if active.next_offset >= active.end_offset {
            self.active = None;
        }

        self.cache.append(chunk_start, &buf[..read]);
        self.pos += read as u64;
        Ok(read)
    }
}

impl Read for HttpFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }

        let mut filled = 0;
        while filled < buf.len() && self.pos < self.len {
            let cached = self.read_from_cache(&mut buf[filled..]);
            if cached > 0 {
                self.trace(format!(
                    "cache hit pos={} bytes={}",
                    self.pos - cached as u64,
                    cached
                ));
                filled += cached;
                continue;
            }

            let active_matches_position = self
                .active
                .as_ref()
                .map(|active| active.next_offset == self.pos)
                .unwrap_or(false);
            if !active_matches_position {
                self.trace(format!(
                    "active stream mismatch at pos={} active_next={}",
                    self.pos,
                    self.active
                        .as_ref()
                        .map(|active| active.next_offset.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ));
                self.reopen_active_range()?;
                if self.active.is_none() {
                    break;
                }
            }

            let read = self.read_from_active(&mut buf[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }

        Ok(filled)
    }
}

impl Seek for HttpFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = seek_target(self.pos, self.len, pos)?;

        if new_pos == self.pos {
            self.trace(format!("seek no-op at pos={}", self.pos));
            return Ok(self.pos);
        }

        if new_pos > self.pos {
            if self.cache.contains(new_pos) {
                self.trace(format!(
                    "seek satisfied from cache old_pos={} new_pos={}",
                    self.pos, new_pos
                ));
                self.pos = new_pos;
                return Ok(self.pos);
            }

            let skip = (new_pos - self.pos) as usize;
            if skip <= SMALL_FORWARD_SEEK_LIMIT && self.skip_with_active_stream(skip)? {
                return Ok(self.pos);
            }

            self.trace(format!(
                "forward seek requires reopen old_pos={} new_pos={} skip={} cache_window={}..{}",
                self.pos,
                new_pos,
                skip,
                self.cache.start,
                self.cache.end()
            ));
        }

        if let Some(active) = &self.active {
            let can_reuse_stream = new_pos == active.next_offset
                || (new_pos < active.next_offset && self.cache.contains(new_pos));
            if !can_reuse_stream {
                self.trace(format!(
                    "dropping active stream on seek old_pos={} new_pos={} active_next={} cache_window={}..{}",
                    self.pos,
                    new_pos,
                    active.next_offset,
                    self.cache.start,
                    self.cache.end()
                ));
                self.active = None;
            }
        }

        self.pos = new_pos;
        Ok(self.pos)
    }
}

#[derive(Debug)]
struct XvdEncryptionInfo {
    full_key: [u8; 32],
    encrypted_sections: Vec<EncryptedSectionInfo>,
}

struct XvdStream<'t> {
    file: &'t mut dyn ReadExactAt,
    offset: u64,
    end_offset: u64,
    pos: u64,

    encryption_info: Option<XvdEncryptionInfo>,
}

impl<'t> Debug for XvdStream<'t> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("XvdStream {} - {}", self.offset, self.end_offset))
    }
}

impl<'t> XvdStream<'t> {
    fn len(&self) -> u64 {
        self.end_offset - self.offset
    }
}

impl<'t>  Read for XvdStream<'t>  {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len() {
            return Ok(0);
        }

        let remaining = usize::try_from(self.len() - self.pos)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "remaining range too large"))?;
        let to_read = remaining.min(buf.len());

        if let Some(encryption_info) = &self.encryption_info {
            for s in &encryption_info.encrypted_sections {
                if self.offset + self.pos >= s.section_offset
                    && self.offset + self.pos < s.section_offset + s.section_length
                {
                    if s.section_offset + s.section_length < self.offset + self.pos + to_read as u64
                    {
                        todo!("Reading outside of the encrypted section in one go is Unsupported");
                    }
                    let mut reader = SectionReader::new(
                        self.file,
                        s.section_offset,
                        s.section_length,
                        s.header_id,
                        s.vduid,
                        encryption_info.full_key,
                        s.data_units.clone(),
                    );
                    reader
                        .read_at(
                            self.offset + self.pos - s.section_offset,
                            &mut buf[..to_read],
                        )
                        .map(|_| {
                            self.pos += to_read as u64;
                            to_read
                        });
                }
            }
        }

        self.file.seek(SeekFrom::Start(self.offset + self.pos))?;
        let read = self.file.read(&mut buf[..to_read])?;
        self.pos += read as u64;
        Ok(read)
    }
}

impl<'t>  Seek for XvdStream<'t>  {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_relative = seek_target(self.pos, self.len(), pos)?;
        self.pos = new_relative;
        Ok(new_relative)
    }
}

impl<'t>  Write for XvdStream<'t>  {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            "XvdStream is read-only",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn extract_ntfs_file<T: Read + Seek>(
    fs: &mut T,
    file: &NtfsFile<'_>,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", output_path.to_string_lossy());
    let mut output_file = std::fs::File::create(output_path)?;

    if let Some(data_item) = file.data(fs, "") {
        let data_item = data_item?;
        let data_attribute = data_item.to_attribute()?;
        let mut data_value = data_attribute.value(fs)?;
        let mut buf = [0u8; 8192];

        loop {
            let bytes_read = data_value.read(fs, &mut buf)?;
            if bytes_read == 0 {
                break;
            }

            output_file.write_all(&buf[..bytes_read])?;
        }

        match data_value {
            ntfs::attribute_value::NtfsAttributeValue::Resident(ntfs_resident_attribute_value) => {
                todo!()
            }
            ntfs::attribute_value::NtfsAttributeValue::NonResident(
                ntfs_non_resident_attribute_value,
            ) => {
                for r in ntfs_non_resident_attribute_value.data_runs() {
                    if let Err(e) = r {
                        return Err(Box::new(e));
                    }
                    let d = r.unwrap();
                    let dp = d.data_position();
                    let len = d.allocated_size();
                    println!("data location {dp} + {len}");
                }
            }
            ntfs::attribute_value::NtfsAttributeValue::AttributeListNonResident(
                ntfs_attribute_list_non_resident_attribute_value,
            ) => todo!(),
        }
    }

    Ok(())
}

fn extract_ntfs_directory<T: Read + Seek>(
    ntfs: &Ntfs,
    fs: &mut T,
    directory: &NtfsFile<'_>,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output_dir)?;

    let index = directory.directory_index(fs)?;
    let mut entries = index.entries();

    while let Some(entry) = entries.next(fs) {
        let entry = entry?;
        let Some(file_name) = entry.key() else {
            continue;
        };
        let file_name = file_name?;
        let name = file_name.name().to_string()?;

        if name == "." || name.starts_with('$') {
            continue;
        }

        let child = entry.to_file(ntfs, fs)?;
        let child_output_path = output_dir.join(&name);

        if file_name.is_directory() {
            extract_ntfs_directory(ntfs, fs, &child, &child_output_path)?;
        } else {
            extract_ntfs_file(fs, &child, &child_output_path)?;
        }
    }

    Ok(())
}

pub struct XvdFile {
    pub content_id: String,
    header: XvdHeader,
    drive_data_offset: u64,
    encrypted_section_infos: Vec<EncryptedSectionInfo>,
}

#[derive(Debug, Clone)]
pub struct FileSegment {
    file_name: String,
    data_offset: u64,
    data_length: u64,
    page_offset: u64,
    page_length: u64,
    keep_encrypted: bool,
}

#[derive(Debug, Clone)]
pub struct EncryptedSectionInfo {
    section_offset: u64,
    section_length: u64,

    header_id: u32,
    vduid: [u8; 8],

    // If integrity is enabled, this must contain one entry per page in the section.
    // If integrity is disabled, use page_in_section as the data unit instead.
    data_units: Option<Vec<u32>>,

    files: Vec<FileSegment>,
    data_hashs: Vec<[u8; 20]>,
    data_hash_infos: Vec<DataHashInfo>,
    first_segment_index: u32,
}

#[derive(Debug, Clone)]
pub struct DataHashInfo {
    data_hash: [u8; 20],
    data_unit: u32,
    data_to_hash_offset: u64,
}

struct SegmentMetadataInfo {
    segment: XvdSegmentMetadataSegment,
}

pub trait ReadExactAt: Read + Seek {
    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }
}

impl<T: Read + Seek> ReadExactAt for T {}

struct PrefixCacheReader<'t> {
    inner: &'t mut dyn ReadExactAt,
    pos: u64,
    len: u64,
    cache: Vec<u8>,
}

impl<'t> PrefixCacheReader<'t> {
    fn new(inner: &'t mut dyn ReadExactAt) -> io::Result<Self> {
        let pos = inner.stream_position()?;
        let len = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(pos))?;

        Ok(Self {
            inner,
            pos,
            len,
            cache: Vec::new(),
        })
    }

    fn ensure_cached(&mut self, end: u64) -> io::Result<()> {
        let target = end.min(self.len).min(PREFIX_CACHE_LIMIT_BYTES);
        let cached_end = self.cache.len() as u64;
        if target <= cached_end {
            return Ok(());
        }

        self.inner.seek(SeekFrom::Start(cached_end))?;
        let missing = usize::try_from(target - cached_end)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "cache extension too large"))?;
        let start = self.cache.len();
        self.cache.resize(start + missing, 0);
        self.inner.read_exact(&mut self.cache[start..])?;
        Ok(())
    }

    fn read_sparse(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || offset >= self.len {
            return Ok(0);
        }

        let available = usize::try_from((self.len - offset).min(buf.len() as u64))
            .map_err(|_| Error::new(ErrorKind::InvalidData, "available sparse range too large"))?;
        self.inner.read_exact_at(&mut buf[..available], offset)?;
        Ok(available)
    }
}

impl Read for PrefixCacheReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }

        let end = self
            .pos
            .checked_add(buf.len() as u64)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "read range overflow"))?;
        self.ensure_cached(end)?;
        let cached_end = self.cache.len() as u64;

        if self.pos >= cached_end {
            let read = self.read_sparse(self.pos, buf)?;
            self.pos += read as u64;
            return Ok(read);
        }

        let cached_available = usize::try_from((cached_end - self.pos).min(buf.len() as u64))
            .map_err(|_| Error::new(ErrorKind::InvalidData, "cached range too large"))?;
        let start = usize::try_from(self.pos)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "offset too large"))?;
        let end = start + cached_available;
        buf[..cached_available].copy_from_slice(&self.cache[start..end]);

        if cached_available == buf.len() {
            self.pos += cached_available as u64;
            return Ok(cached_available);
        }

        let sparse_offset = self.pos + cached_available as u64;
        let sparse_read = self.read_sparse(sparse_offset, &mut buf[cached_available..])?;
        self.pos += (cached_available + sparse_read) as u64;
        Ok(cached_available + sparse_read)
    }
}

impl Seek for PrefixCacheReader<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = seek_target(self.pos, self.len, pos)?;
        self.pos = new_pos;
        Ok(self.pos)
    }
}

pub fn parse_file<T: ReadExactAt>(mut file: T) -> Result<XvdFile, Box<dyn std::error::Error>> {
    let mut header_buffer = [0u8; 4096];
    let mut info_buffer = [0u8; 0xDA8];

    file.read_exact(&mut header_buffer).unwrap();

    let xvd_header: XvdHeader = transmute!(header_buffer);

    // Extracts from header to avoid padding issues
    let format_version = xvd_header.format_version;
    let xvc_length = xvd_header.xvc_data_length;
    let volume_flags = xvd_header.volume_flags;
    let xvc_data_length = xvd_header.xvc_data_length;
    let is_encrypted = xvd_header.is_encrypted();
    let legacy_sector_size = xvd_header.is_legacy_sector_size();
    let _content_types = xvd_header.xvd_content_type;
    let _sector_size = xvd_header.sector_size();
    let _number_of_metadata_pages = xvd_header.number_of_metadata_pages();

    let mdu_offset = xvd_header.mdu_offset();
    let (_hash_tree_levels, hash_tree_page_count) = xvd_header.hash_tree_info();
    let xvc_info_offset = xvd_header.xvc_info_offset(hash_tree_page_count);

    println!("Version: {}", format_version);
    println!("XvcLength: {}", xvc_length);
    println!("volume_flags: 0x{:X}", volume_flags);
    println!("is_encrypted: {}", is_encrypted);
    println!("legacy_sector_size: {}", legacy_sector_size);
    println!("xvc_data_length: {}", xvc_data_length);

    let mut region_headers: Vec<XvcRegionHeader> = Vec::new();
    let mut update_segments: Vec<XvdUpdateSegment> = Vec::new();
    let mut region_specifiers: Vec<XvcRegionSpecifier> = Vec::new();
    let mut region_presence_info: Vec<u8> = Vec::new();

    let mut info: Option<XvcInfo> = None;
    // TODO: Check if we have proper content type
    if xvc_data_length > 0 {
        file.seek(std::io::SeekFrom::Start(xvc_info_offset))
            .expect("Unable to seek");
        file.read_exact(&mut info_buffer).unwrap();
        let xvc_info: XvcInfo = transmute!(info_buffer);
        // info = Some(xvc_info);

        let region_count = xvc_info.region_count;
        let update_segment_count = xvc_info.update_segment_count;
        let region_specifier_count = xvc_info.region_specifier_count;

        if xvc_info.version >= 1 {
            let mut region_header_buf = [0u8; 0x80];
            for _ in 0..region_count {
                file.read_exact(&mut region_header_buf).unwrap();
                let region_header: XvcRegionHeader = transmute!(region_header_buf);
                region_headers.push(region_header);
            }

            let mut update_segment_buf = [0u8; 0xC];
            for _ in 0..update_segment_count {
                file.read_exact(&mut update_segment_buf).unwrap();
                let update_segment: XvdUpdateSegment = transmute!(update_segment_buf);
                update_segments.push(update_segment);
            }

            if xvc_info.version >= 2 {
                let mut region_specifier_buf = [0u8; 0x188];
                for _ in 0..region_specifier_count {
                    file.read_exact(&mut region_specifier_buf).unwrap();
                    let region_specifier: XvcRegionSpecifier = transmute!(region_specifier_buf);
                    region_specifiers.push(region_specifier);
                }

                if xvd_header.mutable_page_count > 0 {
                    file.seek(std::io::SeekFrom::Start(mdu_offset))
                        .expect("Unable to seek");
                    let mut byte = [0; 1];
                    for _ in 0..region_count {
                        file.read_exact(&mut byte).unwrap();
                        region_presence_info.push(byte[0]);
                    }
                }
            }
        }
    }

    let hash_tree_offset = xvd_header.mutable_data_length() + mdu_offset;
    let user_data_offset = if xvd_header.is_data_integrity_enabled() {
        page_number_to_offset(xvd_header.hash_tree_info().1)
    } else {
        0
    } + hash_tree_offset;
    let xvc_info_offset =
        page_number_to_offset(xvd_header.user_data_page_count()) + user_data_offset;
    let dynamic_header_offset =
        page_number_to_offset(xvd_header.xvc_data_page_count()) + xvc_info_offset;
    let drive_data_offset =
        page_number_to_offset(xvd_header.dynamic_header_page_count()) + dynamic_header_offset;
    let _dynamic_base_offset = xvc_info_offset;
    let _static_data_length = if xvd_header.xvd_type == 0 {
        0
    } else {
        panic!("Unsupported XvdType, TODO support Dynamic")
    };

    let mut sfile = file;

    let mut enc_sections: Vec<EncryptedSectionInfo> = vec![];
    for h in &region_headers {
        // let ch = h.clone();
        let key_id = h.key_id;
        let offset = h.offset;
        let length = h.length;
        println!(
            "key_id {} ({} + {} = {})",
            key_id,
            offset,
            length,
            offset + length
        );

        if h.key_id != 0 {
            continue;
        }

        let mut data_units: Vec<u32> = vec![];
        let mut data_hashs: Vec<[u8; 20]> = vec![];
        let mut data_hash_infos: Vec<DataHashInfo> = vec![];
        let start_page = offset_to_page_number(h.offset - user_data_offset);
        let num_pages = bytes_to_pages(length);
        let section_started = Instant::now();
        let r = h.region_id;
        println!(
            "reading {} hash pages for region {} starting at page {}",
            num_pages, r, start_page
        );
        let mut page = 0;
        while page < num_pages {
            let (hash_block, entry_num) = calculate_hash_block_num_for_block_num(
                xvd_header.xvd_type,
                _hash_tree_levels,
                xvd_header.number_of_hashed_pages(),
                start_page + page,
                0,
                false,
                false,
            );
            let mut run_len = 1u64;
            while page + run_len < num_pages {
                let (next_hash_block, next_entry_num) = calculate_hash_block_num_for_block_num(
                    xvd_header.xvd_type,
                    _hash_tree_levels,
                    xvd_header.number_of_hashed_pages(),
                    start_page + page + run_len,
                    0,
                    false,
                    false,
                );
                if next_hash_block != hash_block || next_entry_num != entry_num + run_len {
                    break;
                }
                run_len += 1;
            }

            let hash_entry_offset =
                hash_tree_offset + page_number_to_offset(hash_block) + (entry_num * 0x18);
            let mut hash_entries = vec![0u8; run_len as usize * 0x18];
            sfile
                .read_exact_at(&mut hash_entries, hash_entry_offset)
                .unwrap();

            for (index, entry) in hash_entries.chunks_exact(0x18).enumerate() {
                let page_no = page + index as u64;
                let mut block_hash = [0u8; 0x14];
                block_hash.copy_from_slice(&entry[..0x14]);
                data_hashs.push(block_hash);

                let mut buf = [0u8; 4];
                buf.copy_from_slice(&entry[0x14..0x18]);
                let u = u32::from_le_bytes(buf);
                data_units.push(u);

                let data_to_hash_offset =
                    page_number_to_offset(start_page + page_no) + user_data_offset;

                data_hash_infos.push(DataHashInfo {
                    data_hash: block_hash,
                    data_unit: u,
                    data_to_hash_offset,
                });
            }

            let last_page = page + run_len;
            if http_trace_enabled() && (page == 0 || last_page % 4096 == 0 || last_page == num_pages)
            {
                let r = h.region_id;
                eprintln!(
                    "[xvd] region {} page {}/{} hash_entry_offset={} run_len={} elapsed_ms={}",
                    r,
                    last_page,
                    num_pages,
                    hash_entry_offset,
                    run_len,
                    section_started.elapsed().as_millis()
                );
            }

            if true {
                page += run_len;
                continue;
            }

            let mut buf = [0u8; 4];
            let mut block_hash = [0u8; 0x14];
            let data_to_hash_offset = page_number_to_offset(start_page + page) + user_data_offset;
            let mut block = [0u8; 4096];
            sfile.seek(SeekFrom::Start(data_to_hash_offset));
            sfile.read_exact(&mut block).unwrap();

            let mut sha = sha2::Sha256::new();
            sha.update(block);
            let calculated = sha.finalize();
            if calculated[..0x14] == block_hash {
                println!("page checksum {} ok", start_page + page)
            } else {
                println!("page checksum {} not ok", start_page + page)
            }
            page += run_len;
        }
        let rid = h.region_id;
        println!(
            "finished region {} hash pages in {} ms",
            rid,
            section_started.elapsed().as_millis()
        );

        enc_sections.push(EncryptedSectionInfo {
            section_offset: h.offset,
            section_length: h.length,
            header_id: h.region_id,
            vduid: xvd_header.vduid[..8].try_into().unwrap(),
            data_units: Some(data_units.clone()),
            data_hashs: data_hashs,
            data_hash_infos: data_hash_infos,
            first_segment_index: h.first_segment_index,
            files: vec![],
        });
    }

    let mut user_data_header_buf = [0u8; 128 / 8];
    sfile
        .read_exact_at(&mut user_data_header_buf, user_data_offset)
        .unwrap();
    let user_data_header: XvdUserDataHeader = transmute!(user_data_header_buf);
    if user_data_header.t == 0 {
        let mut off = user_data_offset + user_data_header.length as u64;
        let mut user_data_package_files_header_buf = [0u8; 4224 / 8];
        sfile
            .read_exact_at(&mut user_data_package_files_header_buf, off)
            .unwrap();
        let user_data_package_files_header: XvdUserDataPackageFilesHeader =
            transmute!(user_data_package_files_header_buf);
        let c = user_data_package_files_header.file_count;
        let fullname = user_data_package_files_header.package_full_name;
        println!(
            "package {} / file count {}",
            String::from_utf16(&fullname).unwrap(),
            c
        );
        off += user_data_package_files_header_buf.len() as u64;
        for _ in 0..user_data_package_files_header.file_count {
            let mut user_data_package_files_header_buf = [0u8; 4224 / 8];
            sfile
                .read_exact_at(&mut user_data_package_files_header_buf, off)
                .unwrap();
            let user_data_package_file_entry: XvdUserDataPackageFileEntry =
                transmute!(user_data_package_files_header_buf);
            off += user_data_package_files_header_buf.len() as u64;
            let o = user_data_package_file_entry.offset;
            let s: u32 = user_data_package_file_entry.size;
            let fullname = user_data_package_file_entry.file_path;
            let end = fullname
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(fullname.len());
            let pfull_name: String = String::from_utf16(&fullname[..end]).unwrap();
            println!("file {} / file offset {} size {}", pfull_name, o, s);

            if pfull_name == "SegmentMetadata.bin" {
                let mut buf = [0u8; 800 / 8];
                sfile
                    .read_exact_at(
                        &mut buf,
                        user_data_offset + user_data_header_buf.len() as u64 + o as u64,
                    )
                    .unwrap();
                let segment_header: XvdSegmentMetadataHeader = transmute!(buf);
                let paths_offset = segment_header.header_length as u64
                    + segment_header.segment_count as u64 * 0x10;
                let mut seg_metadata: Vec<u8> = vec![];
                seg_metadata.resize(segment_header.segment_count as usize * 0x10, 0);
                sfile
                    .read_exact_at(
                        &mut seg_metadata,
                        (user_data_offset
                            + user_data_header_buf.len() as u64
                            + o as u64
                            + segment_header.header_length as u64) as u64                    )
                    .unwrap();
                for section in &mut enc_sections {
                    let mut page_offset = section.section_offset.div_ceil(PAGE_SIZE as u64);
                    for segment_no in section.first_segment_index..segment_header.segment_count {
                        let start = segment_no as usize * 0x10;
                        let end = start + 0x10;

                        let bytes: [u8; 0x10] = seg_metadata[start..end].try_into().unwrap();
                        let segment: XvdSegmentMetadataSegment = transmute!(bytes);
                        let s = segment.path_length;
                        let mut buf = vec![0u16, 0];
                        buf.resize(s as usize, 0);
                        sfile
                            .read_exact_at(
                                buf.as_mut_bytes(),
                                (user_data_offset
                                    + o as u64
                                    + user_data_header_buf.len() as u64
                                    + paths_offset
                                    + segment.path_offset as u64)
                                    as u64,
                            )
                            .unwrap();
                        let file_name: String = String::from_utf16(buf.as_slice()).unwrap();
                        println!(
                            "{segment_no}/{page_offset} {} {}",
                            if segment.flags == 1 { "E" } else { " " },
                            file_name
                        );
                        let page_length = if segment.filesize == 0 {
                            1
                        } else {
                            segment.filesize.div_ceil(PAGE_SIZE as u64)
                        };
                        if !(page_offset * (PAGE_SIZE as u64)
                            < section.section_offset + section.section_length)
                        {
                            break;
                        }
                        section.files.push(FileSegment {
                            file_name,
                            data_offset: page_offset * PAGE_SIZE as u64,
                            data_length: segment.filesize,
                            page_offset,
                            page_length,
                            keep_encrypted: segment.flags == 1,
                        });
                        page_offset += page_length;
                    }
                }
            }
        }
    }

    // let gp = gpt::GptConfig::new()
    //     .writable(false)
    //     .logical_block_size(gpt::disk::LogicalBlockSize::Lb4096)
    //     .open_from_device(XvdStream {
    //         file: &mut sfile,
    //         offset: drive_data_offset,
    //         end_offset: drive_data_offset + xvd_header.drive_size,
    //         pos: 0,
    //         encryption_info: None,
    //     })
    //     .unwrap();

    // let mut ntfs_partition = None;
    // for (index, part) in gp.partitions() {
    //     if !part.is_used() {
    //         continue;
    //     }

    //     let part_start = part.bytes_start(*gp.logical_block_size()).unwrap();
    //     let part_len = part.bytes_len(*gp.logical_block_size()).unwrap();
    //     println!(
    //         "#{index}: '{}' start={} len={}",
    //         part.name, part_start, part_len,
    //     );

    //     if ntfs_partition.is_none() {
    //         ntfs_partition = Some((index, part.name.clone(), part_start, part_len));
    //     }
    // }

    // let (_, _, part_start, part_len) = ntfs_partition.expect("no used GPT partition found");
    // let partition_offset = drive_data_offset + part_start;
    // let mut cached_file = PrefixCacheReader::new(&mut sfile)?;
    // cached_file.seek(SeekFrom::Start(partition_offset)).unwrap();

    // let mut fs = XvdStream {
    //     file: &mut cached_file,
    //     offset: partition_offset,
    //     end_offset: partition_offset + part_len,
    //     pos: 0,
    //     // encryption_info: Some(XvdEncryptionInfo {
    //     //     full_key,
    //     //     encrypted_sections: enc_sections,
    //     // }),
    //     encryption_info: None,
    // };
    // // fs.seek(SeekFrom::Start(0)).unwrap();
    // let mut ntfs = Ntfs::new(&mut fs).unwrap();

    // ntfs.read_upcase_table(&mut fs).unwrap();

    // let root = ntfs.root_directory(&mut fs).unwrap();

    // extract_segments(&ntfs, &mut fs, &root, Path::new(""), &mut enc_sections).unwrap();

    Ok(XvdFile {
        content_id: uuid::Uuid::from_bytes_le(xvd_header.vduid).to_string(),
        header: xvd_header,
        drive_data_offset,
        encrypted_section_infos: enc_sections,
    })
}

fn extract_segments<T: Read + Seek>(
    ntfs: &Ntfs,
    fs: &mut T,
    directory: &NtfsFile<'_>,
    output_dir: &Path,
    enc_sections: &mut Vec<EncryptedSectionInfo>
) -> Result<(), Box<dyn std::error::Error>> {
    let index = directory.directory_index(fs)?;
    let mut entries = index.entries();

    while let Some(entry) = entries.next(fs) {
        let entry = entry?;
        let Some(file_name) = entry.key() else {
            continue;
        };
        let file_name = file_name?;
        let name = file_name.name().to_string()?;

        if name == "." || name.starts_with('$') {
            continue;
        }

        let child = entry.to_file(ntfs, fs)?;
        let child_output_path = output_dir.join(&name);

        if file_name.is_directory() {
            extract_segments(ntfs, fs, &child, &child_output_path, enc_sections)?;
        } else {
            // extract_ntfs_file(fs, &child, &child_output_path)?;
            if let Some(data_item) = child.data(fs, "") {
                let data_item = data_item?;
                let data_attribute = data_item.to_attribute()?;
                let data_value = data_attribute.value(fs)?;

                if let ntfs::attribute_value::NtfsAttributeValue::NonResident(ntfs_non_resident_attribute_value) = data_value {
                    let mut data_start: usize = 0;
                    let mut data_size: usize = 0;
                    for run in ntfs_non_resident_attribute_value.data_runs() {
                        let run = run.unwrap();
                        if data_start == 0 {
                            data_start = run.data_position().value().unwrap().get() as usize;
                        } else if data_start + data_size != run.data_position().value().unwrap().get() as usize {
                            todo!("Non continuous NTFS file");
                        }
                        data_size += run.allocated_size() as usize;
                    }
                    for sec in &mut *enc_sections {
                        if data_start >= sec.section_offset as usize && data_start < (sec.section_offset + sec.section_length) as usize {
                            if data_start + data_size > (sec.section_offset + sec.section_length) as usize {
                                todo!("NTFS crosses section");
                            }
                            sec.files.push(FileSegment { file_name: child_output_path.to_string_lossy().into_owned(), data_offset: data_start as u64, data_length: data_size as u64, page_offset: offset_to_page_number(data_start as u64), page_length: offset_to_page_number(data_size as u64), keep_encrypted: false });
                        }
                    }
                }
                // match data_value {
                //     ntfs::attribute_value::NtfsAttributeValue::Resident(ntfs_resident_attribute_value) => todo!(),
                //     ntfs::attribute_value::NtfsAttributeValue::NonResident(ntfs_non_resident_attribute_value) => {
                //         // ntfs_non_resident_attribute_value.data_runs()
                //     },
                //     ntfs::attribute_value::NtfsAttributeValue::AttributeListNonResident(ntfs_attribute_list_non_resident_attribute_value) => todo!(),
                // }
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum FetchReason {
    Missing,
    LengthMismatch,
    ShaMismatch,
    ReadErr,
}

impl FetchReason {
    fn as_str(&self) -> &'static str {
        match self {
            FetchReason::Missing => "Missing",
            FetchReason::LengthMismatch => "LengthMismatch",
            FetchReason::ShaMismatch => "ShaMismatch",
            FetchReason::ReadErr => "ReadErr",
        }
    }
}

struct UnpackPlan<'T> {
    reason: FetchReason,
    section: &'T EncryptedSectionInfo,
    file: &'T FileSegment,
    final_path: PathBuf,
}

pub fn unpack_file(
    xvd: XvdFile,
    path: String,
    destination: String,
    full_key: [u8; 32],
) -> Result<(), Box<dyn std::error::Error>> {
    let block_size = 4096; //xvd.header.block_size;

    let extract_root = PathBuf::from(destination);

    let mut tweak_key = [0u8; 16];
    let mut data_key = [0u8; 16];
    tweak_key.copy_from_slice(&full_key[..16]);
    data_key.copy_from_slice(&full_key[16..]);

    // let enc_s = xvd.encrypted_section_infos.to_vec();

    let mut plan: Vec<UnpackPlan> = vec![];
    for section in &xvd.encrypted_section_infos {
        // let mut xvdStream = XvdStream {
        //     file: sfile.try_clone().unwrap(),
        //     offset: section.section_offset,
        //     end_offset: section.section_offset + section.section_length,
        //     encryption_info: Some(XvdEncryptionInfo {
        //         full_key,
        //         encrypted_sections: enc_s.to_vec(),
        //     }),
        // };
        for fs in &section.files {
            let page_offset = fs.page_offset;
            let page_length = fs.page_length;
            let page_in_section = page_offset - section.section_offset.div_ceil(PAGE_SIZE as u64);

            let path = fs.file_name.replace("\\", "/");
            let final_path = extract_root.join(path);
            let metadata = std::fs::metadata(&final_path);
            if let Err(_) = metadata {
                plan.push(UnpackPlan {
                    reason: FetchReason::Missing,
                    section,
                    file: fs,
                    final_path,
                });
                continue;
            }
            let metadata = metadata.unwrap();
            if metadata.len() != fs.data_length {
                plan.push(UnpackPlan {
                    reason: FetchReason::LengthMismatch,
                    section,
                    file: fs,
                    final_path,
                });
                continue;
            }
            let file: Result<std::fs::File, Error> = std::fs::File::open(&final_path);
            if let Err(_) = file {
                plan.push(UnpackPlan {
                    reason: FetchReason::Missing,
                    section,
                    file: fs,
                    final_path,
                });
                continue;
            }
            let mut file = file.unwrap();

            for p in 0..page_length {
                let mut buf = [0u8; PAGE_SIZE as usize];
                let data_unit = match &section.data_units {
                    Some(units) => *units.get(page_in_section as usize + p as usize).unwrap(),
                    None => page_in_section as u32 + p as u32,
                };
                let len_read: usize;
                if let Err(_) = if p == page_length - 1 {
                    len_read = fs.data_length as usize % PAGE_SIZE as usize;
                    if len_read == 0 {
                        // Do not compare garbage?
                        break;
                    }
                    file.read_exact(&mut buf[..len_read])
                } else {
                    len_read = buf.len();
                    file.read_exact(&mut buf)
                } {
                    plan.push(UnpackPlan {
                        reason: FetchReason::ReadErr,
                        section,
                        file: fs,
                        final_path,
                    });
                    break;
                }

                // let mut expected_buf = [0u8; PAGE_SIZE as usize];
                // xvdStream.seek(SeekFrom::Start(page_in_section * PAGE_SIZE as u64)).unwrap();
                // xvdStream.read_exact(&mut expected_buf).unwrap();

                let encrypted = transform_page_xts(
                    &buf,
                    data_unit,
                    section.header_id,
                    section.vduid,
                    data_key,
                    tweak_key,
                    true,
                )
                .unwrap();

                let mut sha = sha2::Sha256::new();
                sha.update(encrypted);
                let calculated = sha.finalize();

                let info = &section.data_hash_infos[page_in_section as usize + p as usize];

                // let mut block: [u8; 4096] = [0u8; 4096];
                // sfile.read_exact_at(&mut block, info.data_to_hash_offset).unwrap();

                // let decrypted = transform_page_xts(&block, data_unit, section.header_id, section.vduid, data_key, tweak_key, false).unwrap();
                // let reencrypted = transform_page_xts(&decrypted, data_unit, section.header_id, section.vduid, data_key, tweak_key, true).unwrap();

                // if block == reencrypted {
                //     println!("data match")
                // } else {
                //     println!("data mismatch")
                // }
                //
                // let mut sha = sha2::Sha256::new();
                // sha.update(block);
                // let calculated_expected = sha.finalize();

                if calculated[..0x14] != section.data_hashs[page_in_section as usize + p as usize] {
                    println!("BEGIN {} page checksum {} not ok", final_path.display(), p);
                    println!("size {} lenRead {len_read}", fs.data_length);
                    // let mut block: [u8; 4096] = [0u8; 4096];
                    // sfile
                    //     .read_exact_at(&mut block, info.data_to_hash_offset)
                    //     .unwrap();

                    // let decrypted = transform_page_xts(
                    //     &block,
                    //     data_unit,
                    //     section.header_id,
                    //     section.vduid,
                    //     data_key,
                    //     tweak_key,
                    //     false,
                    // )
                    // .unwrap();
                    // let reencrypted = transform_page_xts(
                    //     &decrypted,
                    //     data_unit,
                    //     section.header_id,
                    //     section.vduid,
                    //     data_key,
                    //     tweak_key,
                    //     true,
                    // )
                    // .unwrap();

                    // if block == reencrypted {
                    //     println!("data match")
                    // } else {
                    //     println!("data mismatch")
                    // }
                    // let mut sha = sha2::Sha256::new();
                    // sha.update(block);
                    // let calculated_expected = sha.finalize();
                    // if calculated_expected[..0x14]
                    //     == section.data_hashs[page_in_section as usize + p as usize]
                    // {
                    //     println!("page expected checksum {} ok", p)
                    // } else {
                    //     println!("page expected checksum {} not ok", p)
                    // }
                    // for (i, (&dec, &raw)) in decrypted.iter().zip(buf.iter()).enumerate() {
                    //     println!("{i:08x}: {dec:02x}  {raw:02x}");
                    // }
                    println!("END {} page checksum {} not ok", final_path.display(), p);
                    plan.push(UnpackPlan {
                        reason: FetchReason::ShaMismatch,
                        section,
                        file: fs,
                        final_path,
                    });
                    break;
                }

                // if calculated_expected[..0x14] == section.data_hashs[page_in_section as usize + p as usize] {
                //     println!("page expected checksum {} ok", p)
                // } else {
                //     println!("page expected checksum {} not ok", p)
                // }
            }
        }
    }
    let total_download_bytes: u64 = plan.iter().map(|entry| entry.file.data_length).sum();
    println!(
        "{} files need download ({} bytes)",
        plan.len(),
        total_download_bytes
    );
    let reasons = [
        FetchReason::Missing,
        FetchReason::LengthMismatch,
        FetchReason::ShaMismatch,
        FetchReason::ReadErr,
    ];
    for reason in &reasons {
        let matching: Vec<&UnpackPlan> = plan
            .iter()
            .filter(|entry| entry.reason.as_str() == reason.as_str())
            .collect();
        if matching.is_empty() {
            continue;
        }
        let bytes: u64 = matching.iter().map(|entry| entry.file.data_length).sum();
        println!(
            "{}: {} files ({} bytes)",
            reason.as_str(),
            matching.len(),
            bytes
        );
    }
    let mut largest: Vec<&UnpackPlan> = plan.iter().collect();
    largest.sort_by_key(|entry| std::cmp::Reverse(entry.file.data_length));
    println!("Largest planned downloads:");
    for entry in largest.into_iter().take(20) {
        println!(
            "{} {} bytes {}",
            entry.reason.as_str(),
            entry.file.data_length,
            entry.final_path.display()
        );
    }
    for e in &plan {
        println!("{} {:?}", e.final_path.display(), e.reason);
    }
    return Ok(());
    // let sfile = std::fs::File::open(path)?;
    // let gp = gpt::GptConfig::new()
    //     .writable(false)
    //     .logical_block_size(if block_size == 512 {
    //         gpt::disk::LogicalBlockSize::Lb512
    //     } else if block_size == 4096 {
    //         gpt::disk::LogicalBlockSize::Lb4096
    //     } else {
    //         todo!("unsupported block_size: {}", block_size)
    //     })
    //     .open_from_device(XvdStream {
    //         file: sfile.try_clone().unwrap(),
    //         offset: xvd.drive_data_offset,
    //         end_offset: xvd.drive_data_offset + xvd.header.drive_size,
    //         encryption_info: None,
    //     })
    //     .unwrap();

    // let mut ntfs_partition = None;
    // for (index, part) in gp.partitions() {
    //     if !part.is_used() {
    //         continue;
    //     }

    //     let part_start = part.bytes_start(*gp.logical_block_size()).unwrap();
    //     let part_len = part.bytes_len(*gp.logical_block_size()).unwrap();
    //     println!(
    //         "#{index}: '{}' start={} len={}",
    //         part.name, part_start, part_len,
    //     );

    //     if ntfs_partition.is_none() {
    //         ntfs_partition = Some((index, part.name.clone(), part_start, part_len));
    //     }
    // }

    // let (_, _, part_start, part_len) = ntfs_partition.expect("no used GPT partition found");
    // let partition_offset = xvd.drive_data_offset + part_start;

    // let mut fs = XvdStream {
    //     file: sfile.try_clone().unwrap(),
    //     offset: partition_offset,
    //     end_offset: partition_offset + part_len,
    //     encryption_info: Some(XvdEncryptionInfo {
    //         full_key,
    //         encrypted_sections: xvd.encrypted_section_infos,
    //     }),
    // };
    // fs.seek(SeekFrom::Start(0)).unwrap();
    // let mut ntfs = Ntfs::new(&mut fs).unwrap();

    // ntfs.read_upcase_table(&mut fs).unwrap();

    // let root = ntfs.root_directory(&mut fs).unwrap();
    // let extract_root = PathBuf::from(destination);
    // println!("extracting data directory to {}", extract_root.display());
    // extract_ntfs_directory(&ntfs, &mut fs, &root, &extract_root)?;
    // Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HttpFile, PAGE_SIZE, ReadAheadBuffer, XvdFile, parse_content_range_total, parse_file, seek_target};
    use std::collections::BTreeMap;
    use std::io::{ErrorKind, SeekFrom};
    use std::path::Path;

    fn encrypted_file_hashes(xvd: &XvdFile) -> BTreeMap<(String, u64), Vec<[u8; 20]>> {
        let mut files = BTreeMap::new();

        for section in &xvd.encrypted_section_infos {
            let section_page_start = section.section_offset.div_ceil(PAGE_SIZE as u64);
            for file in &section.files {
                let page_in_section = file.page_offset - section_page_start;
                let start = page_in_section as usize;
                let end = start + file.page_length as usize;
                files.insert(
                    (file.file_name.clone(), file.data_length),
                    section.data_hashs[start..end].to_vec(),
                );
            }
        }

        files
    }

    #[test]
    fn content_range_total_is_parsed() {
        assert_eq!(parse_content_range_total("bytes 0-0/1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
        assert_eq!(parse_content_range_total("invalid"), None);
    }

    #[test]
    fn read_ahead_buffer_retains_tail() {
        let mut cache = ReadAheadBuffer::new(4);
        cache.append(0, b"ab");
        cache.append(2, b"cd");
        cache.append(4, b"ef");

        let mut out = [0u8; 4];
        let read = cache.read_into(2, &mut out);
        assert_eq!(read, 4);
        assert_eq!(&out, b"cdef");
    }

    #[test]
    fn seek_target_rejects_out_of_bounds() {
        assert_eq!(seek_target(5, 10, SeekFrom::Current(-3)).unwrap(), 2);
        assert_eq!(seek_target(5, 10, SeekFrom::End(-2)).unwrap(), 8);
        assert_eq!(seek_target(5, 10, SeekFrom::Start(10)).unwrap(), 10);
        assert_eq!(
            seek_target(5, 10, SeekFrom::Start(11)).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    #[ignore = "requires network access and a local 1.26.2101.0 package at a fixed path"]
    fn probe_encrypted_file_hash_stability_for_same_license() {
        let remote_url = "http://assets1.xboxlive.com/12/480484ea-7b1c-4443-b152-411e07e1329d/7792d9ce-355a-493c-afbd-768f4a77c3b0/1.26.3005.0.48bff00d-cff7-4432-8d1e-662d92be1b14/Microsoft.MinecraftUWP_1.26.3005.0_x64__8wekyb3d8bbwe.msixvc";
        let local_path = Path::new(
            "/Users/christopher/Documents/minecraft/xodus/MICROSOFT.MINECRAFTUWP_1.26.2101.0_x64__8wekyb3d8bbwe.msixvc",
        );

        assert!(
            local_path.exists(),
            "missing local package at {}",
            local_path.display()
        );

        let remote = parse_file(
            HttpFile::with_readahead(remote_url, 8 * 1024 * 1024)
                .expect("failed to open remote package"),
        )
        .expect("failed to parse remote package");
        let local = parse_file(
            std::fs::File::open(local_path).expect("failed to open local package"),
        )
        .expect("failed to parse local package");

        let remote_files = encrypted_file_hashes(&remote);
        let local_files = encrypted_file_hashes(&local);
        let mut local_files_by_name: BTreeMap<&str, Vec<(u64, &Vec<[u8; 20]>)>> = BTreeMap::new();
        for ((name, size), hashes) in &local_files {
            local_files_by_name
                .entry(name.as_str())
                .or_default()
                .push((*size, hashes));
        }

        let mut same_name_and_size = 0usize;
        let mut stable_files = 0usize;
        let mut stable_pages = 0usize;
        let mut compared_pages = 0usize;
        let mut mismatches: Vec<(String, u64, usize, usize)> = vec![];
        let mut same_name_different_size = 0usize;
        let mut reusable_pages_different_size = 0usize;
        let mut compared_pages_different_size = 0usize;
        let mut different_size_reuse: Vec<(String, u64, u64, usize, usize)> = vec![];

        for ((name, size), remote_hashes) in &remote_files {
            let Some(local_hashes) = local_files.get(&(name.clone(), *size)) else {
                let Some(local_candidates) = local_files_by_name.get(name.as_str()) else {
                    continue;
                };
                let Some((local_size, local_hashes)) =
                    local_candidates.iter().find(|(local_size, _)| local_size != size)
                else {
                    continue;
                };

                let matching_pages = remote_hashes
                    .iter()
                    .zip(local_hashes.iter())
                    .filter(|(left, right)| left == right)
                    .count();
                same_name_different_size += 1;
                reusable_pages_different_size += matching_pages;
                compared_pages_different_size += remote_hashes.len().min(local_hashes.len());
                different_size_reuse.push((
                    name.clone(),
                    *size,
                    *local_size,
                    matching_pages,
                    remote_hashes.len().min(local_hashes.len()),
                ));
                continue;
            };

            same_name_and_size += 1;
            compared_pages += remote_hashes.len().min(local_hashes.len());

            if remote_hashes == local_hashes {
                stable_files += 1;
                stable_pages += remote_hashes.len();
            } else {
                let matching_pages = remote_hashes
                    .iter()
                    .zip(local_hashes.iter())
                    .filter(|(left, right)| left == right)
                    .count();
                stable_pages += matching_pages;
                mismatches.push((name.clone(), *size, matching_pages, remote_hashes.len()));
            }
        }

        mismatches.sort_by(|left, right| {
            let left_ratio = left.2 as f64 / left.3 as f64;
            let right_ratio = right.2 as f64 / right.3 as f64;
            right_ratio
                .partial_cmp(&left_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.1.cmp(&left.1))
        });
        different_size_reuse.sort_by_key(|(_, remote_size, _, _, _)| std::cmp::Reverse(*remote_size));

        println!(
            "remote content_id={} local content_id={}",
            remote.content_id, local.content_id
        );
        println!(
            "same-name same-size files={} stable_files={} stable_pages={}/{}",
            same_name_and_size, stable_files, stable_pages, compared_pages
        );
        println!(
            "same-name different-size files={} reusable_pages={}/{}",
            same_name_different_size, reusable_pages_different_size, compared_pages_different_size
        );
        println!("largest unstable files:");
        for (name, size, matching_pages, total_pages) in mismatches.into_iter() {
            println!("{size} bytes {matching_pages}/{total_pages} pages {name}");
        }
        println!("largest different-size reusable files:");
        for (name, remote_size, local_size, matching_pages, total_pages) in
            different_size_reuse.into_iter()
        {
            println!(
                "remote={remote_size} local={local_size} {matching_pages}/{total_pages} pages {name}"
            );
        }

        assert!(
            same_name_and_size > 0,
            "no same-name same-size files found to compare"
        );
    }
}
