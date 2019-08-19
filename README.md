# awmp

[![Docs](https://docs.rs/awmp/badge.svg)](https://docs.rs/crate/awmp/)
[![Crates.io](https://img.shields.io/crates/v/awmp.svg)](https://crates.io/crates/awmp)

A convenience library for working with multipart/form-data in [`actix-web`](https://docs.rs/actix-web) 1.x.

This library uses [`actix-multipart`](https://docs.rs/actix-multipart/0.1.3/actix_multipart/) internally, and is not a replacement
for `actix-multipart`. It saves multipart file data to tempfiles and collects text data, handling all blocking I/O operations.

Provides some configuration options in [PartsConfig](struct.PartsConfig.html):

* **text_limit**: Any text field data larger than this number of bytes will be saved as a tempfile
* **file_limit**: Any file field data larger than this number of bytes will be discarded/ignored
* **file_fields**: Always treat fields with these names as file fields
* **temp_dir**: Use this folder as the tmp directory, rather than `tempfile`'s default


## Usage

```rust
use actix_web::FromRequest;

pub fn upload(mut parts: awmp::Parts) -> Result<actix_web::HttpResponse, actix_web::Error> {
    let qs = parts.texts.to_query_string();

    let file_parts = parts
        .files
        .remove("file")
        .pop()
        .and_then(|f| f.persist("/tmp").ok())
        .map(|f| format!("File uploaded to: {}", f.display()))
        .unwrap_or_default();

    let body = [format!("Text parts: {}", &qs), file_parts].join(", ");

    Ok(actix_web::HttpResponse::Ok().body(body))
}

fn main() -> Result<(), Box<::std::error::Error>> {
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

Current version: 0.3.0

License: MIT
