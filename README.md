# awmp

[![Docs](https://docs.rs/awmp/badge.svg)](https://docs.rs/crate/awmp/)
[![Crates.io](https://img.shields.io/crates/v/awmp.svg)](https://crates.io/crates/awmp)

A convenience library for working with multipart/form-data in [`actix-web`](https://docs.rs/actix-web) 1.x or 2.x.

This library uses [`actix-multipart`](https://docs.rs/actix-multipart) internally, and is not a replacement
for `actix-multipart`. It saves multipart file data to tempfiles and collects text data, handling all blocking I/O operations.

Provides some configuration options in [PartsConfig](struct.PartsConfig.html):

* **text_limit**: Any text field data larger than this number of bytes will be saved as a tempfile
* **file_limit**: Any file field data larger than this number of bytes will be discarded/ignored
* **file_fields**: Treat fields with these names as file fields
* **text_fields**: Treat fields with these names as text fields
* **temp_dir**: Use this folder as the tmp directory, rather than `tempfile`'s default

## Usage

This crate supports both major versions of `actix-web`, 1.x and 2.x. It supports 2.x by default.

To use with `actix-web` 1.x, add the following to your `Cargo.toml`:

```toml
awmp = { version = "0.5", default-features = false, features = ["v1"] }
```

### Example

```rust
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

Current version: 0.5.0

License: MIT
