/* copyright Remi Bernotavicius 2020 */

use http_io::client::HttpClient;
use http_io::protocol::{HttpBody, OutgoingBody};
use http_io::url::Url;
use indicatif::{ProgressBar, ProgressStyle};
use std::convert::Infallible;
use std::fmt;
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

#[derive(Debug)]
enum Location {
    Remote(Url),
    Local(PathBuf),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(p) => write!(f, "{}", p.to_string_lossy()),
            Self::Remote(u) => write!(f, "{}", u),
        }
    }
}

impl std::str::FromStr for Location {
    type Err = Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if let Ok(p) = s.parse() {
            Ok(Self::Remote(p))
        } else {
            Ok(Self::Local(PathBuf::from(s)))
        }
    }
}

impl Location {
    fn is_dir(&self) -> bool {
        match self {
            Self::Local(p) => p.is_dir(),
            Self::Remote(u) => u.path.trailing_slash(),
        }
    }

    fn push(&mut self, component: &str) {
        match self {
            Self::Local(p) => p.push(component),
            Self::Remote(u) => u.path.push(component),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Local(p) => p.file_name().unwrap().to_string_lossy().into(),
            Self::Remote(u) => u
                .path
                .components()
                .last()
                .unwrap_or("index.html".into())
                .into(),
        }
    }
}

struct CopyContext {
    http_client: HttpClient<TcpStream>,
}

impl CopyContext {
    fn new() -> Self {
        Self {
            http_client: HttpClient::<TcpStream>::new(),
        }
    }
}

trait StreamSize {
    fn stream_size(&self) -> Option<u64>;
}

trait CopySource<'a> {
    type Stream: io::Read + StreamSize + 'a;
    fn open_for_read(&self, context: &'a mut CopyContext) -> Result<Self::Stream>;
}

trait CopySink {
    type Stream: io::Write;
    fn open_for_write(&self, context: &mut CopyContext) -> Result<Self::Stream>;
}

impl<R: io::Read> StreamSize for HttpBody<R> {
    fn stream_size(&self) -> Option<u64> {
        self.content_length()
    }
}

impl<'a> CopySource<'a> for Url {
    type Stream = HttpBody<&'a mut TcpStream>;
    fn open_for_read(&self, context: &'a mut CopyContext) -> Result<Self::Stream> {
        Ok(context.http_client.get(self.clone())?.finish()?.body)
    }
}

impl CopySink for Url {
    type Stream = OutgoingBody<TcpStream>;
    fn open_for_write(&self, _context: &mut CopyContext) -> Result<Self::Stream> {
        unimplemented!()
    }
}

impl StreamSize for File {
    fn stream_size(&self) -> Option<u64> {
        unimplemented!()
    }
}

impl<'a> CopySource<'a> for PathBuf {
    type Stream = File;
    fn open_for_read(&self, _context: &'a mut CopyContext) -> Result<Self::Stream> {
        unimplemented!()
    }
}

impl CopySink for PathBuf {
    type Stream = File;
    fn open_for_write(&self, _context: &mut CopyContext) -> Result<Self::Stream> {
        Ok(File::create(self)?)
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "wcp", about = "Web Copy. Copies URLs to local destinations")]
struct Options {
    source: Location,
    destination: Location,
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

fn do_io_copy<SOURCE, SINK>(source: SOURCE, destination: SINK) -> Result<()>
where
    for<'a> SOURCE: CopySource<'a>,
    SINK: CopySink,
{
    let mut source_context = CopyContext::new();
    let mut destination_context = CopyContext::new();

    let mut source_stream = source.open_for_read(&mut source_context)?;
    let mut destination_stream = destination.open_for_write(&mut destination_context)?;

    let mut progress = match source_stream.stream_size() {
        Some(length) => ProgressBar::new(length),
        None => ProgressBar::new_spinner(),
    };

    progress.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {bytes}/{total_bytes} ({bytes_per_sec}) (eta {eta})"),
    );

    io_copy_with_progress(&mut source_stream, &mut destination_stream, &mut progress)?;

    Ok(())
}

fn do_copy(source: Location, mut destination: Location) -> Result<()> {
    if destination.is_dir() {
        destination.push(&source.name());
    }

    println!("copying {} to {}", source, destination);

    match (source, destination) {
        (Location::Local(source), Location::Local(destination)) => do_io_copy(source, destination),
        (Location::Local(source), Location::Remote(destination)) => do_io_copy(source, destination),
        (Location::Remote(source), Location::Local(destination)) => do_io_copy(source, destination),
        (Location::Remote(source), Location::Remote(destination)) => {
            do_io_copy(source, destination)
        }
    }
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

    do_copy(Location::Remote(url), Location::Local(local_path.clone())).unwrap();

    let contents = std::fs::read_to_string(local_path).unwrap();
    assert_eq!(contents, "file_data");
}

fn main() -> Result<()> {
    let options = Options::from_args();
    do_copy(options.source, options.destination)
}
