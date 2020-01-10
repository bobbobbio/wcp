/* copyright Remi Bernotavicius 2020 */

use http_io::client::HttpClient;
use http_io::url::Url;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::File;
use std::io;
use std::net::TcpStream;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug)]
enum Error {
    Http(http_io::error::Error),
    Io(io::Error),
}

type Result<T> = std::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<http_io::error::Error> for Error {
    fn from(e: http_io::error::Error) -> Self {
        Self::Http(e)
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "wcp", about = "Web Copy. Copies URLs to local destinations")]
struct Options {
    url: http_io::url::Url,
    output: PathBuf,
}

const DEFAULT_BUF_SIZE: usize = 8 * 1024;

pub fn io_copy_with_progress<R: ?Sized, W: ?Sized>(
    reader: &mut R,
    writer: &mut W,
    progress: &mut ProgressBar,
) -> io::Result<u64>
where
    R: io::Read,
    W: io::Write,
{
    let mut buf = [0u8; DEFAULT_BUF_SIZE];
    let mut written = 0;
    loop {
        let len = match reader.read(&mut buf) {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..len])?;
        written += len as u64;
        progress.inc(len as u64);
    }
}

fn do_copy(url: Url, output: PathBuf) -> Result<()> {
    let mut client = HttpClient::<TcpStream>::new();

    let destination = if output.is_dir() {
        let name = url.path.components().last().unwrap_or("index.html");
        output.join(name)
    } else {
        output
    };

    let mut output_file = File::create(&destination)?;
    println!("copying {} to {}", url, destination.to_string_lossy());

    let mut body = client.get(url)?.finish()?.body;

    let mut progress = match body.content_length() {
        Some(length) => ProgressBar::new(length),
        None => ProgressBar::new_spinner(),
    };

    progress.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {bytes}/{total_bytes} ({bytes_per_sec}) (eta {eta})"),
    );

    io_copy_with_progress(&mut body, &mut output_file, &mut progress)?;

    Ok(())
}

#[cfg(test)]
use http_io::{
    protocol::{HttpResponse, HttpStatus},
    server::{HttpRequestHandler, HttpServer},
};

#[cfg(test)]
struct TestHandler(String);

#[cfg(test)]
impl<I: io::Read> HttpRequestHandler<I> for TestHandler {
    type Error = http_io::error::Error;

    fn get(&mut self, _uri: String) -> http_io::error::Result<HttpResponse<Box<dyn io::Read>>> {
        Ok(HttpResponse::from_string(HttpStatus::OK, &self.0))
    }
}

/// End-to-end integration test of downloading a file from an HTTP server.
#[test]
fn test_do_copy() {
    let server_socket = std::net::TcpListener::bind("localhost:0").unwrap();
    let server_address = server_socket.local_addr().unwrap();
    let handler = TestHandler("file_data".into());
    let mut server = HttpServer::new(server_socket, handler);
    std::thread::spawn(move || server.serve_one());

    let url = format!("http://localhost:{}/", server_address.port())
        .parse()
        .unwrap();
    let temporary_file = tempfile::NamedTempFile::new().unwrap();
    let local_path = temporary_file.path().to_path_buf();

    do_copy(url, local_path.clone()).unwrap();

    let contents = std::fs::read_to_string(local_path).unwrap();
    assert_eq!(contents, "file_data");
}

fn main() -> Result<()> {
    let options = Options::from_args();
    do_copy(options.url, options.output)
}
