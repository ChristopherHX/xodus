use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use ntfs::{Ntfs, NtfsFile, NtfsReadSeek};
use rsa::sha2::{self, Digest};
use smbioslib::CpuStatus::UserDisabled;
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncSeekExt},
};
use zerocopy::{IntoBytes, transmute};

use crate::models::xvd::{PAGE_SIZE, XvdSegmentMetadataHeader, XvdSegmentMetadataSegment, XvdUserDataHeader, XvdUserDataPackageFileEntry, XvdUserDataPackageFilesHeader};
use crate::xvd::crypt::{SectionReader, transform_page_xts};
use crate::xvd::math::{
    bytes_to_pages, calculate_hash_block_num_for_block_num, offset_to_page_number,
};
use crate::{
    models::xvd::{XvcInfo, XvcRegionHeader, XvcRegionSpecifier, XvdHeader, XvdUpdateSegment},
    xvd::math::page_number_to_offset,
};

#[derive(Debug)]
struct XvdEncryptionInfo {
    full_key: [u8; 32],
    encrypted_sections: Vec<EncryptedSectionInfo>,
}

#[derive(Debug)]
struct XvdStream {
    file: std::fs::File,
    offset: u64,
    end_offset: u64,

    encryption_info: Option<XvdEncryptionInfo>,
}

impl XvdStream {
    fn len(&self) -> u64 {
        self.end_offset - self.offset
    }

    fn current_relative_pos(&mut self) -> std::io::Result<u64> {
        let absolute = self.file.stream_position()?;
        absolute
            .checked_sub(self.offset)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "stream before virtual start"))
    }
}

impl Read for XvdStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let current = self.current_relative_pos()?;
        if current >= self.len() {
            return Ok(0);
        }

        let remaining = usize::try_from(self.len() - current)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "remaining range too large"))?;
        let to_read = remaining.min(buf.len());

        if let Some(encryption_info) = &self.encryption_info {
            for s in &encryption_info.encrypted_sections {
                if self.offset + current >= s.section_offset
                    && self.offset + current < s.section_offset + s.section_length
                {
                    if s.section_offset + s.section_length < self.offset + current + to_read as u64
                    {
                        todo!("Reading outside of the encrypted section in one go is Unsupported");
                    }
                    let mut reader = SectionReader::new(
                        &self.file,
                        s.section_offset,
                        s.section_length,
                        s.header_id,
                        s.vduid,
                        encryption_info.full_key,
                        s.data_units.clone(),
                    );
                    return reader
                        .read_at(
                            self.offset + current - s.section_offset,
                            &mut buf[..to_read],
                        )
                        .map(|_| to_read);
                }
            }
        }

        self.file.read(&mut buf[..to_read])
    }
}

impl Seek for XvdStream {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_relative = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(delta) => {
                let current = self.current_relative_pos()?;
                if delta >= 0 {
                    current.checked_add(delta as u64)
                } else {
                    current.checked_sub(delta.unsigned_abs())
                }
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid relative seek"))?
            }
            SeekFrom::End(delta) => {
                let len = self.len();
                if delta >= 0 {
                    len.checked_add(delta as u64)
                } else {
                    len.checked_sub(delta.unsigned_abs())
                }
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid end-relative seek"))?
            }
        };

        if new_relative > self.len() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "seek past virtual device end",
            ));
        }

        self.file
            .seek(SeekFrom::Start(self.offset + new_relative))?;
        Ok(new_relative)
    }
}

impl Write for XvdStream {
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
            ntfs::attribute_value::NtfsAttributeValue::Resident(ntfs_resident_attribute_value) => todo!(),
            ntfs::attribute_value::NtfsAttributeValue::NonResident(ntfs_non_resident_attribute_value) => {
                for r in ntfs_non_resident_attribute_value.data_runs() {
                    if let Err(e) = r {
                        return Err(Box::new(e));
                    }
                    let d = r.unwrap();
                    let dp = d.data_position();
                    let len = d.allocated_size();
                    println!("data location {dp} + {len}");
                }
            },
            ntfs::attribute_value::NtfsAttributeValue::AttributeListNonResident(ntfs_attribute_list_non_resident_attribute_value) => todo!(),
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

pub async fn parse_file(path: String) -> Result<XvdFile, Box<dyn std::error::Error>> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path.clone())
        .await
        .expect("Unable to open file");
    let mut header_buffer = [0u8; 4096];
    let mut info_buffer = [0u8; 0xDA8];

    file.read_exact(&mut header_buffer).await.unwrap();

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
            .await
            .expect("Unable to seek");
        file.read_exact(&mut info_buffer).await.unwrap();
        let xvc_info: XvcInfo = transmute!(info_buffer);
        // info = Some(xvc_info);

        let region_count = xvc_info.region_count;
        let update_segment_count = xvc_info.update_segment_count;
        let region_specifier_count = xvc_info.region_specifier_count;

        if xvc_info.version >= 1 {
            let mut region_header_buf = [0u8; 0x80];
            for _ in 0..region_count {
                file.read_exact(&mut region_header_buf).await.unwrap();
                let region_header: XvcRegionHeader = transmute!(region_header_buf);
                region_headers.push(region_header);
            }

            let mut update_segment_buf = [0u8; 0xC];
            for _ in 0..update_segment_count {
                file.read_exact(&mut update_segment_buf).await.unwrap();
                let update_segment: XvdUpdateSegment = transmute!(update_segment_buf);
                update_segments.push(update_segment);
            }

            if xvc_info.version >= 2 {
                let mut region_specifier_buf = [0u8; 0x188];
                for _ in 0..region_specifier_count {
                    file.read_exact(&mut region_specifier_buf).await.unwrap();
                    let region_specifier: XvcRegionSpecifier = transmute!(region_specifier_buf);
                    region_specifiers.push(region_specifier);
                }

                if xvd_header.mutable_page_count > 0 {
                    file.seek(std::io::SeekFrom::Start(mdu_offset))
                        .await
                        .expect("Unable to seek");
                    let mut byte = [0; 1];
                    for _ in 0..region_count {
                        file.read_exact(&mut byte).await.unwrap();
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

    let sfile = std::fs::File::open(path).unwrap();

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
        let mut data_hash_infos : Vec<DataHashInfo> = vec![];
        let start_page = offset_to_page_number(h.offset - user_data_offset);
        let num_pages = bytes_to_pages(length);
        for page in 0..num_pages {
            let mut buf = [0u8; 4];
            let (hash_block, entry_num) = calculate_hash_block_num_for_block_num(
                xvd_header.xvd_type,
                _hash_tree_levels,
                xvd_header.number_of_hashed_pages(),
                start_page + page,
                0,
                false,
                false,
            );
            let hash_entry_offset =
                hash_tree_offset + page_number_to_offset(hash_block) + (entry_num * 0x18);
            let read_offset= hash_entry_offset + 0x14;
            sfile.read_exact_at(&mut buf, read_offset).unwrap();
            let u = u32::from_le_bytes(buf);
            data_units.push(u);

            let mut block_hash = [0u8; 0x14];
            sfile.read_exact_at(&mut block_hash, hash_entry_offset).unwrap();
            data_hashs.push(block_hash);

            let data_to_hash_offset = page_number_to_offset(start_page + page) + user_data_offset;

            data_hash_infos.push(DataHashInfo { data_hash: block_hash, data_unit: u, data_to_hash_offset: data_to_hash_offset });

            if true {
                continue;
            }

            let mut block = [0u8; 4096];
            sfile.read_exact_at(&mut block, data_to_hash_offset).unwrap();

            let mut sha = sha2::Sha256::new();
            sha.update(block);
            let calculated = sha.finalize();
            if calculated[..0x14] == block_hash {
                println!("page checksum {} ok", start_page + page)
            } else {
                println!("page checksum {} not ok", start_page + page)
            }
        }

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

    let mut user_data_header_buf = [0u8; 128/8];
    sfile.read_exact_at(&mut user_data_header_buf, user_data_offset).unwrap();
    let user_data_header : XvdUserDataHeader = transmute!(user_data_header_buf);
    if user_data_header.t == 0 {
        let mut off = user_data_offset + user_data_header.length as u64;
        let mut user_data_package_files_header_buf = [0u8; 4224/8];
        sfile.read_exact_at(&mut user_data_package_files_header_buf, off).unwrap();
        let user_data_package_files_header : XvdUserDataPackageFilesHeader = transmute!(user_data_package_files_header_buf);
        let c = user_data_package_files_header.file_count;
        let fullname = user_data_package_files_header.package_full_name;
        println!("package {} / file count {}", String::from_utf16(&fullname).unwrap(), c);
        off += user_data_package_files_header_buf.len() as u64;
        for _ in 0..user_data_package_files_header.file_count {
            let mut user_data_package_files_header_buf = [0u8; 4224/8];
            sfile.read_exact_at(&mut user_data_package_files_header_buf, off).unwrap();
            let user_data_package_file_entry : XvdUserDataPackageFileEntry = transmute!(user_data_package_files_header_buf);
            off += user_data_package_files_header_buf.len() as u64;
            let o  = user_data_package_file_entry.offset;
            let s: u32  = user_data_package_file_entry.size;
            let fullname = user_data_package_file_entry.file_path;
            let end = fullname.iter().position(|&c| c == 0).unwrap_or(fullname.len());
            let pfull_name: String = String::from_utf16(&fullname[..end]).unwrap();
            println!("file {} / file offset {} size {}", pfull_name, o, s);

            if pfull_name == "SegmentMetadata.bin" {                
                let mut buf = [0u8; 800/8];
                sfile.read_exact_at(&mut buf, user_data_offset + user_data_header_buf.len() as u64 + o as u64).unwrap();
                let segment_header : XvdSegmentMetadataHeader = transmute!(buf);
                let paths_offset = segment_header.header_length as u64 + segment_header.segment_count as u64 * 0x10;
                for section in &mut enc_sections {
                    let mut page_offset = section.section_offset.div_ceil(PAGE_SIZE as u64);
                    for segment_no in section.first_segment_index..segment_header.segment_count {
                        let mut buf = [0u8; 128/8];
                        sfile.read_exact_at(&mut buf, (user_data_offset + user_data_header_buf.len() as u64 + o as u64 + segment_header.header_length as u64) as u64 + segment_no as u64 * 0x10).unwrap();
                        let segment : XvdSegmentMetadataSegment = transmute!(buf);
                        let s = segment.path_length;
                        let mut buf = vec![0u16, 0];
                        buf.resize(s as usize, 0);
                        sfile.read_exact_at(buf.as_mut_bytes(), (user_data_offset + o as u64 + user_data_header_buf.len() as u64 + paths_offset + segment.path_offset as u64) as u64).unwrap();
                        let file_name: String = String::from_utf16(buf.as_slice()).unwrap();
                        println!("{segment_no}/{page_offset} {} {}", if segment.flags == 1 { "E" } else { " " }, file_name);
                        let page_length = if segment.filesize == 0 { PAGE_SIZE as u64 } else { segment.filesize.div_ceil(PAGE_SIZE as u64) };
                        if !(page_offset * (PAGE_SIZE as u64) < section.section_offset + section.section_length) {
                            break;
                        }
                        section.files.push(FileSegment { file_name, data_offset: page_offset * PAGE_SIZE as u64, data_length: segment.filesize, page_offset, page_length, keep_encrypted: segment.flags == 1 });
                        page_offset += page_length;
                    }
                }
            }
        }
    }

    Ok(XvdFile {
        content_id: uuid::Uuid::from_bytes_le(xvd_header.vduid).to_string(),
        header: xvd_header,
        drive_data_offset,
        encrypted_section_infos: enc_sections,
    })
}

#[derive(Debug)]
enum FetchReason {
    Missing,
    LengthMismatch,
    ShaMismatch,
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
    let sfile = std::fs::File::open(path)?;
    let block_size = 4096; //xvd.header.block_size;

    let extract_root = PathBuf::from(destination);

    let mut tweak_key = [0u8; 16];
    let mut data_key = [0u8; 16];
    tweak_key.copy_from_slice(&full_key[..16]);
    data_key.copy_from_slice(&full_key[16..]);

    let enc_s = xvd.encrypted_section_infos.to_vec();

    let mut plan: Vec<UnpackPlan> = vec![]; 
    for section in &xvd.encrypted_section_infos {
        let mut xvdStream = XvdStream {
            file: sfile.try_clone().unwrap(),
            offset: section.section_offset,
            end_offset: section.section_offset + section.section_length,
            encryption_info: Some(XvdEncryptionInfo {
                full_key,
                encrypted_sections: enc_s.to_vec(),
            }),
        };
        for fs in &section.files {
            let page_offset = fs.page_offset;
            let page_length = fs.page_length;
            let page_in_section = page_offset - section.section_offset.div_ceil(PAGE_SIZE as u64);
            
            let path = fs.file_name.replace("\\", "/");
            let final_path = extract_root.join(path);
            let metadata = std::fs::metadata(&final_path);
            if let Err(_) = metadata {
                plan.push(UnpackPlan { reason: FetchReason::Missing, section, file: fs, final_path });
                continue;
            }
            let metadata = metadata.unwrap();
            if metadata.len() != fs.data_length {
                plan.push(UnpackPlan { reason: FetchReason::LengthMismatch, section, file: fs, final_path });
                continue;
            }
            let file: Result<std::fs::File, Error> = std::fs::File::open(&final_path);
            if let Err(_) = file {
                plan.push(UnpackPlan { reason: FetchReason::Missing, section, file: fs, final_path });
                continue;
            }
            let mut file = file.unwrap();

            for p in 0..page_length {
                let mut buf = [0u8; PAGE_SIZE as usize];
                let data_unit = match &section.data_units {
                    Some(units) => *units
                        .get(page_in_section as usize + p as usize).unwrap(),
                    None => page_in_section as u32 + p as u32,
                };
                if let Err(_) = if p == page_length - 1 {
                    file.read(&mut buf[..(fs.data_length as usize % PAGE_SIZE as usize)]).map(|_| ())
                } else {
                    file.read_exact(&mut buf)
                } {
                    plan.push(UnpackPlan { reason: FetchReason::ShaMismatch, section, file: fs, final_path});
                    break;
                }

                // let mut expected_buf = [0u8; PAGE_SIZE as usize];
                // xvdStream.seek(SeekFrom::Start(page_in_section * PAGE_SIZE as u64)).unwrap();
                // xvdStream.read_exact(&mut expected_buf).unwrap();

                let encrypted = transform_page_xts(&buf, data_unit, section.header_id, section.vduid, data_key, tweak_key, true).unwrap();

                let mut sha = sha2::Sha256::new();
                sha.update(encrypted);
                let calculated = sha.finalize();

                // let info = &section.data_hash_infos[page_in_section as usize + p as usize];

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

                if calculated[..0x14] == section.data_hashs[page_in_section as usize + p as usize] {
                    println!("page checksum {} ok", p);
                } else {
                    println!("page checksum {} not ok", p);
                    plan.push(UnpackPlan { reason: FetchReason::ShaMismatch, section, file: fs, final_path});
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
    for e in &plan {
        println!("{} {:?}", e.final_path.display(), e.reason);
    }
    return Ok(());
    let gp = gpt::GptConfig::new()
        .writable(false)
        .logical_block_size(if block_size == 512 {
            gpt::disk::LogicalBlockSize::Lb512
        } else if block_size == 4096 {
            gpt::disk::LogicalBlockSize::Lb4096
        } else {
            todo!("unsupported block_size: {}", block_size)
        })
        .open_from_device(XvdStream {
            file: sfile.try_clone().unwrap(),
            offset: xvd.drive_data_offset,
            end_offset: xvd.drive_data_offset + xvd.header.drive_size,
            encryption_info: None,
        })
        .unwrap();

    let mut ntfs_partition = None;
    for (index, part) in gp.partitions() {
        if !part.is_used() {
            continue;
        }

        let part_start = part.bytes_start(*gp.logical_block_size()).unwrap();
        let part_len = part.bytes_len(*gp.logical_block_size()).unwrap();
        println!(
            "#{index}: '{}' start={} len={}",
            part.name, part_start, part_len,
        );

        if ntfs_partition.is_none() {
            ntfs_partition = Some((index, part.name.clone(), part_start, part_len));
        }
    }

    let (_, _, part_start, part_len) = ntfs_partition.expect("no used GPT partition found");
    let partition_offset = xvd.drive_data_offset + part_start;

    let mut fs = XvdStream {
        file: sfile.try_clone().unwrap(),
        offset: partition_offset,
        end_offset: partition_offset + part_len,
        encryption_info: Some(XvdEncryptionInfo {
            full_key,
            encrypted_sections: xvd.encrypted_section_infos,
        }),
    };
    fs.seek(SeekFrom::Start(0)).unwrap();
    let mut ntfs = Ntfs::new(&mut fs).unwrap();

    ntfs.read_upcase_table(&mut fs).unwrap();

    let root = ntfs.root_directory(&mut fs).unwrap();
    let extract_root = PathBuf::from(destination);
    println!("extracting data directory to {}", extract_root.display());
    extract_ntfs_directory(&ntfs, &mut fs, &root, &extract_root)?;
    Ok(())
}
