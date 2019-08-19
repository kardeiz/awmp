pub fn upload(parts: awmp::Parts) -> Result<actix_web::HttpResponse, actix_web::Error> {
    let qs = parts.texts.to_query_string();

    let files = parts
        .files
        .into_inner()
        .into_iter()
        .map(|(name, tf)| tf.persist("/tmp").map(|f| (name, f)))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, f)| format!("{}: {}", name, f.display()))
        .collect::<Vec<_>>()
        .join(", ");

    let body = format!("Text parts: {}, File parts: {}\r\n", &qs, &files);

    Ok(actix_web::HttpResponse::Ok().body(body))
}

fn main() -> Result<(), Box<::std::error::Error>> {
    actix_web::HttpServer::new(move || {
        actix_web::App::new().route("/", actix_web::web::post().to(upload))
    })
    .bind("0.0.0.0:3000")?
    .run()?;

    Ok(())
}
