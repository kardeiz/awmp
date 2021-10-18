/*!
A convenience library for working with multipart/form-data in [`actix-web`](https://docs.rs/actix-web) 1.x, 2.x, or 3.x.

This library uses [`actix-multipart`](https://docs.rs/actix-multipart) internally, and is not a replacement
for `actix-multipart`. It saves multipart file data to tempfiles and collects text data, handling all blocking I/O operations.

Provides some configuration options in [PartsConfig](struct.PartsConfig.html):

* **text_limit**: Any text field data larger than this number of bytes will be saved as a tempfile
* **file_limit**: Any file field data larger than this number of bytes will be discarded/ignored
* **file_fields**: Treat fields with these names as file fields
* **text_fields**: Treat fields with these names as text fields
* **temp_dir**: Use this folder as the tmp directory, rather than `tempfile`'s default

# Usage

This crate supports both major versions of `actix-web`, 1.x, 2.x, and 3.x. It supports 3.x by default.

To use with `actix-web` 1.x, add the following to your `Cargo.toml`:

```toml
awmp = { version = "0.6", default-features = false, features = ["v1"] }
```

## Example

```rust,no_run
use actix_web::{web, App, Error, FromRequest, HttpResponse, HttpServer};

async fn upload(mut parts: awmp::Parts) -> Result<actix_web::HttpResponse, actix_web::Error> {
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

#[actix_rt::main]
async fn main() -> Result<(), std::io::Error> {
    actix_web::HttpServer::new(move || {
        actix_web::App::new()
            .data(awmp::Parts::configure(|cfg| cfg.with_file_limit(1_000_000)))
            .route("/", actix_web::web::post().to(upload))
    })
    .bind("0.0.0.0:3000")?
    .run()
    .await
}
```
*/

use bytes::Bytes;

use tempfile::NamedTempFile;

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

#[cfg(feature = "v1")]
pub(crate) use actix_web_v1 as actix_web;

#[cfg(feature = "v1")]
pub(crate) use actix_multipart_v01 as actix_multipart;

#[cfg(feature = "v1")]
pub mod v1;

#[cfg(feature = "v2")]
pub(crate) use actix_web_v2 as actix_web;

#[cfg(feature = "v2")]
pub(crate) use actix_multipart_v02 as actix_multipart;

#[cfg(feature = "v2")]
#[path = "v2_3.rs"]
pub mod v2;

#[cfg(feature = "v3")]
pub(crate) use actix_web_v3 as actix_web;

#[cfg(feature = "v3")]
pub(crate) use actix_multipart_v03 as actix_multipart;

#[cfg(feature = "v3")]
#[path = "v2_3.rs"]
pub mod v3;

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

impl AsRef<NamedTempFile> for File {
    fn as_ref(&self) -> &NamedTempFile {
        &self.inner
    }
}

impl AsMut<NamedTempFile> for File {
    fn as_mut(&mut self) -> &mut NamedTempFile {
        &mut self.inner
    }
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

    /// Returns `HashMap`  of field names and values
    /// NOTE: this will discard the first of multiple values for a key
    pub fn as_hash_map(&self) -> HashMap<&str, &str> {
        self.as_pairs().into_iter().collect()
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

    #[deprecated(since = "0.5.4", note = "Please use the 'persist_in' function instead")]
    /// Persist the tempfile to an existing directory. Uses the sanitized file name and returns
    /// the full path
    ///
    /// NOTE: Because of how temporary file is stored, it cannot be persisted across filesystems.
    /// Also neither the file contents nor the containing directory are
    /// synchronized, so the update may not yet have reached the disk when
    /// `persist` returns.
    pub fn persist<P: AsRef<Path>>(self, dir: P) -> Result<PathBuf, Error> {
        let new_path = dir.as_ref().join(&self.sanitized_file_name);
        self.inner.persist(&new_path).map(|_| new_path).map_err(Error::TempFilePersistError)
    }

    /// Persist the tempfile to an existing directory. Uses the sanitized file name and returns
    /// the full path
    ///
    /// NOTE: Because of how temporary file is stored, it cannot be persisted across filesystems.
    /// Also neither the file contents nor the containing directory are
    /// synchronized, so the update may not yet have reached the disk when
    /// `persist_in` returns.
    pub fn persist_in<P: AsRef<Path>>(self, dir: P) -> Result<PathBuf, Error> {
        let new_path = dir.as_ref().join(&self.sanitized_file_name);
        self.inner.persist(&new_path).map(|_| new_path).map_err(Error::TempFilePersistError)
    }

    /// Persist the tempfile at the specified file path.
    ///
    /// NOTE: Because of how temporary file is stored, it cannot be persisted across filesystems.
    /// Also neither the file contents nor the containing directory are
    /// synchronized, so the update may not yet have reached the disk when
    /// `persist_at` returns.
    pub fn persist_at<P: AsRef<Path>>(self, path: P) -> Result<std::fs::File, Error> {
        self.inner.persist(path).map_err(Error::TempFilePersistError)
    }
}

#[cfg(unix)]
impl File {
    /// Persist the tempfile with specific permissions on Unix
    ///
    /// NOTE: Because of how temporary file is stored, it cannot be persisted across filesystems.
    /// Also neither the file contents nor the containing directory are
    /// synchronized, so the update may not yet have reached the disk when
    /// `persist_with_permissions` returns.
    pub fn persist_with_permissions<P: AsRef<Path>>(
        self,
        dir: P,
        mode: u32,
    ) -> Result<PathBuf, Error> {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(self.inner.path(), permissions).map_err(Error::Io)?;
        let new_path = dir.as_ref().join(&self.sanitized_file_name);
        self.persist_in(&new_path)
    }

    /// Persist the tempfile with 644 permissions on Unix
    ///
    /// NOTE: Because of how temporary file is stored, it cannot be persisted across filesystems.
    /// Also neither the file contents nor the containing directory are
    /// synchronized, so the update may not yet have reached the disk when
    /// `persist_with_open_permissions` returns.
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
