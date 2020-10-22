use actix_web_v3::{web, App, Error, FromRequest, HttpResponse, HttpServer};

pub async fn upload(parts: awmp::Parts) -> Result<HttpResponse, Error> {
    let qs = parts.texts.to_query_string();

    let files = parts
        .files
        .into_inner()
        .into_iter()
        .flat_map(|(name, res_tf)| res_tf.map(|x| (name, x)))
        .map(|(name, tf)| tf.persist_in(std::env::temp_dir()).map(|f| (name, f)))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, f)| format!("{}: {}", name, f.display()))
        .collect::<Vec<_>>()
        .join(", ");

    let body = format!("Text parts: {}, File parts: {}\r\n", &qs, &files);

    Ok(HttpResponse::Ok().body(body))
}

#[actix_rt::main]
async fn main() -> Result<(), std::io::Error> {
    HttpServer::new(move || {
        App::new()
            .data(awmp::Parts::configure(|cfg| cfg.with_file_limit(100000)))
            .route("/", web::post().to(upload))
    })
    .bind("0.0.0.0:3000")?
    .run()
    .await
}
