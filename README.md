# awmp

An easy to use wrapper around multipart fields for Actix web.

Use like:

```rust,ignore
pub fn upload(mut parts: awmp::Parts) -> Result<HttpResponse, ::actix_web::Error> {
    // Serialize parts to query string for easier usage with e.g., serde_urlencoded
    let qs = parts.texts.to_query_string();

    // Use remove to extract files by field name
    // Persist the files to a directory of your choosing    
    let file_paths = parts.files.remove("upload").into_iter()
        .map(|x| x.persist("/tmp") )
        .collect::<Result<Vec<_>, _>>();
    // ...
}