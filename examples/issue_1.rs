use actix_web::FromRequest;

pub fn upload(parts: awmp::Parts) -> Result<actix_web::HttpResponse, actix_web::Error> {
    let files = parts.files.into_inner();

    let file_or_err = files
        .into_iter()
        .filter(|(k, _)| k == "file")
        .map(|(_, v)| v)
        .next()
        .ok_or_else(|| actix_web::error::ErrorBadRequest("Must upload file"))?;

    match file_or_err {
        Ok(file) => {
            let filename = file.sanitized_file_name();
            // Do something with `file`
            Ok(actix_web::HttpResponse::Ok().body("File was processed"))
        }
        Err(e) => Err(actix_web::error::ErrorBadRequest(e)),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    actix_web::HttpServer::new(move || {
        actix_web::App::new()
            .data(awmp::Parts::configure(|cfg| cfg.with_file_limit(10)))
            .route("/", actix_web::web::post().to(upload))
    })
    .bind("0.0.0.0:3000")?
    .run()?;

    Ok(())
}
