extern crate actix_web;
extern crate awmp;
extern crate futures;

use futures::future::Future;

use actix_web::{
    dev, error, http, middleware, multipart, server, App, Error, FromRequest, FutureResponse,
    HttpMessage, HttpRequest, HttpResponse
};

pub fn upload(req: HttpRequest<()>) -> FutureResponse<HttpResponse> {
    Box::new(awmp::Parts::extract(&req).and_then(|parts| {
        println!("{:?}", &parts);
        Ok(HttpResponse::Ok().body("THANKS"))
    }))
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
