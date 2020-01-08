/*!
A convenience library for working with multipart/form-data in [`actix-web`](https://docs.rs/actix-web) 1.x.

This library uses [`actix-multipart`](https://docs.rs/actix-multipart/0.1.3/actix_multipart/) internally, and is not a replacement
for `actix-multipart`. It saves multipart file data to tempfiles and collects text data, handling all blocking I/O operations.

Provides some configuration options in [PartsConfig](struct.PartsConfig.html):

* **text_limit**: Any text field data larger than this number of bytes will be saved as a tempfile
* **file_limit**: Any file field data larger than this number of bytes will be discarded/ignored
* **file_fields**: Treat fields with these names as file fields
* **text_fields**: Treat fields with these names as text fields
* **temp_dir**: Use this folder as the tmp directory, rather than `tempfile`'s default

# Usage

```rust,no_run
use actix_web::FromRequest;

pub fn upload(mut parts: awmp::Parts) -> Result<actix_web::HttpResponse, actix_web::Error> {
    let qs = parts.texts.to_query_string();

    let file_parts = parts
        .files
        .take("file")
        .pop()
        .and_then(|f| f.persist("/tmp").ok())
        .map(|f| format!("File uploaded to: {}", f.display()))
        .unwrap_or_default();

    let body = [format!("Text parts: {}", &qs), file_parts].join(", ");

    Ok(actix_web::HttpResponse::Ok().body(body))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    actix_web::HttpServer::new(move || {
        actix_web::App::new()
            .data(awmp::Parts::configure(|cfg| cfg.with_file_limit(1_000_000)))
            .route("/", actix_web::web::post().to(upload))
    })
    .bind("0.0.0.0:3000")?
    .run()?;

    Ok(())
}
```
*/

use actix_multipart::{Field, Multipart};
use actix_web_v1::{dev, error, http, web, FromRequest, HttpRequest};
use bytes::Bytes;
use futures::{
    future::{self, Either},
    Future, IntoFuture, Stream,
};
use tempfile::NamedTempFile;

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

/// Error container
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    TempFilePersistError(tempfile::PersistError),
    FileTooLarge { limit: usize, file_name: Option<String> },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(ref x) => x.fmt(f),
            Error::TempFilePersistError(ref x) => x.fmt(f),
            Error::FileTooLarge { limit, ref file_name } => {
                if let Some(ref file_name) = file_name {
                    write!(f, "File is too large (limit: {} bytes): {}", limit, file_name)
                } else {
                    write!(f, "File is too large (limit: {} bytes)", limit)
                }
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(ref x) => Some(x),
            Error::TempFilePersistError(ref x) => Some(x),
            _ => None,
        }
    }
}

/// The parts of a multipart/form-data request
#[derive(Debug)]
pub struct Parts {
    pub texts: TextParts,
    pub files: FileParts,
}

/// The text parts of a multipart/form-data request
#[derive(Debug)]
pub struct TextParts(Vec<(String, Bytes)>);

/// The file parts of a multipart/form-data request
#[derive(Debug)]
pub struct FileParts(Vec<(String, Result<File, Error>)>);

/// A tempfile wrapper that includes the original filename
#[derive(Debug)]
pub struct File {
    inner: NamedTempFile,
    original_file_name: Option<String>,
    sanitized_file_name: String,
}

impl TextParts {
    pub fn into_inner(self) -> Vec<(String, Bytes)> {
        self.0
    }

    /// Returns the field names and values as `&str`s. If values are non-UTF8, use
    /// `into_inner` to access values
    pub fn as_pairs(&self) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .flat_map(|(key, val)| std::str::from_utf8(val).map(|val| (key.as_str(), val)))
            .collect()
    }

    /// Re-encodes the string-like text parts to a query string
    pub fn to_query_string(&self) -> String {
        let mut qs = url::form_urlencoded::Serializer::new(String::new());

        for (key, val) in self
            .0
            .iter()
            .flat_map(|(key, val)| std::str::from_utf8(val).map(|val| (key.as_str(), val)))
        {
            qs.append_pair(&key, &val);
        }

        qs.finish()
    }
}

impl FileParts {
    pub fn into_inner(self) -> Vec<(String, Result<File, Error>)> {
        self.0
    }

    /// Get the first non-error file for given name
    pub fn first(&self, key: &str) -> Option<&File> {
        self.0.iter().filter(|(k, _)| k.as_str() == key).flat_map(|(_, v)| v.as_ref()).next()
    }

    /// Returns any files for the given name and removes them from the container
    #[deprecated(note = "Please use `take` instead")]
    pub fn remove(&mut self, key: &str) -> Vec<File> {
        self.take(key)
    }

    pub fn take(&mut self, key: &str) -> Vec<File> {
        let mut taken = Vec::with_capacity(self.0.len());
        let mut untaken = Vec::with_capacity(self.0.len());

        for (k, v) in self.0.drain(..) {
            if k == key && v.is_ok() {
                taken.push(v.unwrap());
            } else {
                untaken.push((k, v));
            }
        }

        self.0 = untaken;

        taken
    }
}

impl File {
    pub fn into_inner(self) -> NamedTempFile {
        self.inner
    }

    /// The filename provided in the multipart/form-data request
    pub fn original_file_name(&self) -> Option<&str> {
        self.original_file_name.as_ref().map(|x| x.as_str())
    }

    /// The sanitized version of the original file name, or generated name if none provided
    pub fn sanitized_file_name(&self) -> &str {
        &self.sanitized_file_name
    }

    /// Persist the tempfile to an existing directory. Uses the sanitized file name and returns
    /// the full path
    pub fn persist<P: AsRef<Path>>(self, dir: P) -> Result<PathBuf, Error> {
        let new_path = dir.as_ref().join(&self.sanitized_file_name);
        self.inner.persist(&new_path).map(|_| new_path).map_err(Error::TempFilePersistError)
    }
}

#[cfg(unix)]
impl File {
    /// Persist the tempfile with specific permissions on Unix
    pub fn persist_with_permissions<P: AsRef<Path>>(
        self,
        dir: P,
        mode: u32,
    ) -> Result<PathBuf, Error> {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(self.inner.path(), permissions).map_err(Error::Io)?;
        let new_path = dir.as_ref().join(&self.sanitized_file_name);
        self.inner.persist(&new_path).map(|_| new_path).map_err(Error::TempFilePersistError)
    }

    /// Persist the tempfile with 644 permissions on Unix
    pub fn persist_with_open_permissions<P: AsRef<Path>>(self, dir: P) -> Result<PathBuf, Error> {
        self.persist_with_permissions(dir, 0o644)
    }
}

/// `FromRequest` configurator
#[derive(Default, Debug, Clone)]
pub struct PartsConfig {
    text_limit: Option<usize>,
    file_limit: Option<usize>,
    file_fields: Option<Vec<String>>,
    text_fields: Option<Vec<String>>,
    temp_dir: Option<PathBuf>,
}

impl PartsConfig {
    /// Any text fields above this limit will be converted to file fields
    pub fn with_text_limit(mut self, text_limit: usize) -> Self {
        self.text_limit = Some(text_limit);
        self
    }

    /// Any file fields above this limit will be ignored
    pub fn with_file_limit(mut self, file_limit: usize) -> Self {
        self.file_limit = Some(file_limit);
        self
    }

    /// Any form names that should be interpreted as files
    pub fn with_file_fields(mut self, file_fields: Vec<String>) -> Self {
        self.file_fields = Some(file_fields);
        self
    }

    /// Any form names that should be interpreted as inline texts
    pub fn with_text_fields(mut self, text_fields: Vec<String>) -> Self {
        self.text_fields = Some(text_fields);
        self
    }

    /// To use a different location than the tempfile default
    pub fn with_temp_dir<I: Into<PathBuf>>(mut self, temp_dir: I) -> Self {
        self.temp_dir = Some(temp_dir.into());
        self
    }
}

impl FromRequest for Parts {
    type Error = actix_web_v1::Error;
    type Future = Box<dyn Future<Item = Self, Error = Self::Error>>;
    type Config = PartsConfig;

    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let opt_cfg = req.get_app_data::<PartsConfig>();

        let rt = Multipart::from_request(req, payload)
            .into_future()
            .map(|mp| mp.map_err(error::ErrorInternalServerError))
            .flatten_stream()
            .map(move |field| handle_field(opt_cfg.clone(), field).into_stream())
            .flatten()
            .collect()
            .map(|parts| {
                let mut texts = Vec::with_capacity(parts.len());
                let mut files = Vec::with_capacity(parts.len());
                for (name, p) in parts.into_iter() {
                    match p {
                        Part::Text(s) => {
                            texts.push((name, s));
                        }
                        Part::File(f) => {
                            files.push((name, f));
                        }
                    }
                }
                Parts { texts: TextParts(texts), files: FileParts(files) }
            });

        Box::new(rt)
    }
}

#[derive(Debug)]
enum Part {
    Text(Bytes),
    File(Result<File, Error>),
}

#[derive(Debug)]
enum Buffer {
    Cursor(Cursor<Vec<u8>>),
    File(NamedTempFile),
}

struct FileTooLarge {
    limit: usize,
}

fn new_temp_file(
    opt_cfg: Option<web::Data<PartsConfig>>,
) -> impl Future<Item = NamedTempFile, Error = error::Error> {
    web::block(move || match opt_cfg.as_ref().and_then(|x| x.temp_dir.as_ref()) {
        Some(temp_dir) => NamedTempFile::new_in(temp_dir),
        _ => NamedTempFile::new(),
    })
    .map_err(error::ErrorInternalServerError)
}

fn handle_field(
    opt_cfg: Option<web::Data<PartsConfig>>,
    field: Field,
) -> impl Future<Item = (String, Part), Error = error::Error> {
    let mut name_opt = None;
    let mut file_name_opt = None;

    for param in field.content_disposition().into_iter().flat_map(|x| x.parameters) {
        match param {
            http::header::DispositionParam::Name(s) => {
                name_opt = Some(s);
            }
            http::header::DispositionParam::Filename(s) => {
                file_name_opt = Some(s);
            }
            _ => {}
        }
    }

    let name = match name_opt {
        Some(s) => s,
        None => {
            return Either::A(future::err(error::ErrorInternalServerError(
                "Field name is required",
            )));
        }
    };

    let mime_type = field.content_type().clone();

    let marked_as_file = opt_cfg
        .as_ref()
        .iter()
        .map(|x| x.file_fields.iter().flatten())
        .flatten()
        .any(|x| x == &name);

    let marked_as_text = opt_cfg
        .as_ref()
        .iter()
        .map(|x| x.text_fields.iter().flatten())
        .flatten()
        .any(|x| x == &name);

    let buffer_fut = match file_name_opt.as_ref() {
        Some(_) if !marked_as_text => Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File)),
        None if marked_as_file => Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File)),
        _ => Either::B(future::ok(Buffer::Cursor(Cursor::new(Vec::new())))),
    };

    let rt =
        future::loop_fn(Either::A(buffer_fut.map(|buffer| (field, buffer, 0))), move |state| {
            let opt_cfg = opt_cfg.clone();
            state.and_then(move |(stream, buffer, mut len)| {
                let opt_cfg = opt_cfg.clone();
                stream.into_future().map_err(|(e, _)| error::ErrorInternalServerError(e)).and_then(
                    move |(bytes, new_stream)| match bytes {
                        Some(bytes) => {
                            let opt_cfg = opt_cfg.clone();

                            len += bytes.len();

                            let mut opt_cursor = None;

                            let buffer_fut = if opt_cfg
                                .as_ref()
                                .and_then(|x| x.text_limit)
                                .map(|x| len > x)
                                .unwrap_or(false)
                            {
                                match buffer {
                                    Buffer::Cursor(cursor) => {
                                        opt_cursor = Some(cursor);
                                        Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File))
                                    }
                                    x => Either::B(future::ok(x)),
                                }
                            } else {
                                Either::B(future::ok(buffer))
                            };

                            Either::A(buffer_fut.map(move |buffer| match buffer {
                                Buffer::Cursor(mut cursor) => {
                                    if let Err(e) = cursor.write_all(bytes.as_ref()) {
                                        return future::Loop::Break(future::err(
                                            error::ErrorInternalServerError(e),
                                        ));
                                    }
                                    future::Loop::Continue(Either::B(Either::A(future::ok((
                                        new_stream,
                                        Buffer::Cursor(cursor),
                                        len,
                                    )))))
                                }
                                Buffer::File(mut file) => {
                                    if let Some(limit) = opt_cfg.as_ref().and_then(|x| x.file_limit)
                                    {
                                        if len > limit {
                                            return future::Loop::Break(future::ok(Either::A(
                                                FileTooLarge { limit },
                                            )));
                                        }
                                    }

                                    let rt = web::block(move || {
                                        let cursor_bytes = opt_cursor
                                            .as_ref()
                                            .map(|x| x.get_ref().as_ref())
                                            .unwrap_or_default();

                                        file.write_all(cursor_bytes)
                                            .and_then(|_| file.write_all(bytes.as_ref()))
                                            .map(|_| Buffer::File(file))
                                    })
                                    .map(move |buffer| (new_stream, buffer, len))
                                    .map_err(error::ErrorInternalServerError);
                                    future::Loop::Continue(Either::B(Either::B(rt)))
                                }
                            }))
                        }
                        None => Either::B(future::ok(future::Loop::Break(future::ok(Either::B(
                            buffer,
                        ))))),
                    },
                )
            })
        })
        .flatten()
        .map(move |buffer| match buffer {
            Either::B(Buffer::Cursor(cursor)) => {
                (name, Part::Text(Bytes::from(cursor.into_inner())))
            }
            Either::B(Buffer::File(file)) => {
                let sanitized_file_name = match file_name_opt {
                    Some(ref s) => sanitize_filename::sanitize(s),
                    None => {
                        let uuid = uuid::Uuid::new_v4().to_simple();
                        match mime_guess::get_mime_extensions(&mime_type).and_then(|x| x.first()) {
                            Some(ext) => format!("{}.{}", uuid, ext),
                            None => uuid.to_string(),
                        }
                    }
                };
                (
                    name,
                    Part::File(Ok(File {
                        inner: file,
                        sanitized_file_name,
                        original_file_name: file_name_opt,
                    })),
                )
            }
            Either::A(FileTooLarge { limit }) => {
                (name, Part::File(Err(Error::FileTooLarge { limit, file_name: file_name_opt })))
            }
        });

    Either::B(rt)
}
