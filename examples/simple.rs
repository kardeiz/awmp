extern crate actix_web;
extern crate awmp;
extern crate futures;

use futures::future::Future;

use actix_web::{
    dev, error, http, middleware, multipart, server, App, Error, FromRequest, FutureResponse,
    HttpMessage, HttpRequest, HttpResponse
};

pub fn upload(mut parts: awmp::Parts) -> Result<HttpResponse, ::actix_web::Error> {
    let qs = parts.texts.to_query_string();

    println!("Text parts as query string: {:?}", qs);

    let files = parts.files.remove("upload");

    for mut file in files {
        println!("File name: {:?}", &file.file_name());

        let mut written_file = file.persist("/tmp/").unwrap();

        ::std::io::Seek::seek(&mut written_file, std::io::SeekFrom::Start(0));
        let mut buf = String::new();
        ::std::io::Read::read_to_string(&mut written_file, &mut buf);
        println!("String contents of file: {:?}", &buf);
    }

    Ok(HttpResponse::Ok().body("THANKS"))
}

fn main() -> Result<(), Box<::std::error::Error>> {
    server::new(|| {
        App::with_state(()).resource("/", |r| {
            r.method(http::Method::POST).with(upload);
        })
    }).bind("127.0.0.1:3000")?
    .run();

    Ok(())
}
