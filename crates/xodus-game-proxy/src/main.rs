
use std::io::Error;
use std::process::Stdio;

use tokio::fs::File;
use tokio::io::{self, AsyncReadExt};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::pipe::pipe;
use tokio::process::Command;

#[cfg(target_os = "linux")]
fn get_runtime_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").expect("Runtime dir not set")
}

#[cfg(target_os = "macos")]
fn get_runtime_dir() -> String {
    "/tmp/".to_string()
}

#[tokio::main]
async fn main() -> io::Result<()> {
    eprintln!("starting proxy");
    let runtime_dir: String = get_runtime_dir();
    let socket_path = format!("{runtime_dir}/xodus.sock");

    let stream = UnixStream::connect(&socket_path).await?;
    let (mut socket_read, mut socket_write) = stream.into_split();

    eprintln!("connected??");
    let mut wn = Command::new("wine")
        .arg("wine-side.exe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let pid = wn.id().unwrap();

    let mut stdin_to_socket = tokio::spawn(async move {
        io::copy(&mut wn.stdout.unwrap(), &mut socket_write).await?;
        Ok::<_, std::io::Error>(())
    });

    let mut socket_to_stdout = tokio::spawn(async move {
        io::copy(&mut socket_read, &mut wn.stdin.unwrap()).await?;
        Ok::<_, Error>(())
    });

    tokio::select! {
        res = &mut stdin_to_socket => res.expect("stdin forwarding task panicked")?,
        res = &mut socket_to_stdout => res.expect("stdout forwarding task panicked")?,
    }
    // let status = wn.wait().await.unwrap();

    Ok(())
}
