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
