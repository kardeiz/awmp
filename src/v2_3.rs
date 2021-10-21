use super::*;
use actix_multipart::{Field, Multipart};
use actix_web::{dev, error, http, web, Error as ActixWebError, FromRequest, HttpRequest};
use futures_v03::{
    future::{Future, TryFutureExt},
    stream::TryStreamExt,
};

impl FromRequest for Parts {
    type Error = ActixWebError;
    type Future = std::pin::Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;
    type Config = PartsConfig;

    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let opt_cfg = req.app_data::<web::Data<PartsConfig>>().cloned();

        Box::pin(Multipart::from_request(req, payload).and_then(move |mp| {
            mp.map_err(error::ErrorInternalServerError)
                .and_then(move |field| handle_field(opt_cfg.clone(), field))
                .try_collect::<Vec<_>>()
                .map_ok(|parts| {
                    let mut texts = Vec::with_capacity(parts.len());
                    let mut files = Vec::with_capacity(parts.len());
                    for (name, p) in parts.into_iter() {
                        match p {
                            Part::Text(s) => {
                                texts.push((name, s));
                            }
                            Part::File(f) => {
                                files.push((name, f));
                            }
                        }
                    }
                    Parts { texts: TextParts(texts), files: FileParts(files) }
                })
        }))
    }
}

async fn new_temp_file(
    opt_cfg: Option<web::Data<PartsConfig>>,
) -> Result<NamedTempFile, error::Error> {
    Ok(web::block(move || match opt_cfg.as_ref().and_then(|x| x.temp_dir.as_ref()) {
        Some(temp_dir) => NamedTempFile::new_in(temp_dir),
        _ => NamedTempFile::new(),
    })
    .map_err(error::ErrorInternalServerError)
    .await?)
}

async fn handle_field(
    opt_cfg: Option<web::Data<PartsConfig>>,
    mut field: Field,
) -> Result<(String, Part), error::Error> {
    let mut name_opt = None;
    let mut file_name_opt = None;

    for param in field.content_disposition().into_iter().flat_map(|x| x.parameters) {
        match param {
            http::header::DispositionParam::Name(s) => {
                name_opt = Some(s);
            }
            http::header::DispositionParam::Filename(s) => {
                file_name_opt = Some(s);
            }
            _ => {}
        }
    }

    let name = match name_opt {
        Some(s) => s,
        None => {
            return Err(error::ErrorInternalServerError("Field name is required"));
        }
    };

    let mime_type = field.content_type().clone();

    let marked_as_file = opt_cfg
        .as_ref()
        .iter()
        .map(|x| x.file_fields.iter().flatten())
        .flatten()
        .any(|x| x == &name);

    let marked_as_text = opt_cfg
        .as_ref()
        .iter()
        .map(|x| x.text_fields.iter().flatten())
        .flatten()
        .any(|x| x == &name);

    let mut buffer = match file_name_opt.as_ref() {
        Some(_) if !marked_as_text => new_temp_file(opt_cfg.clone()).map_ok(Buffer::File).await?,
        None if marked_as_file => new_temp_file(opt_cfg.clone()).map_ok(Buffer::File).await?,
        _ => Buffer::Cursor(Cursor::new(Vec::new())),
    };

    let mut len = 0;
    let mut file_too_large = None;

    while let Some(bytes) = field.try_next().await? {
        len += bytes.len();

        let mut opt_cursor = None;

        if opt_cfg.as_ref().and_then(|x| x.text_limit).map(|x| len > x).unwrap_or(false) {
            buffer = match buffer {
                Buffer::Cursor(cursor) => {
                    opt_cursor = Some(cursor);
                    new_temp_file(opt_cfg.clone()).map_ok(Buffer::File).await?
                }
                x => x,
            };
        }

        if let Some(limit) = opt_cfg.as_ref().and_then(|x| x.file_limit) {
            if let Buffer::File(_) = buffer {
                if len > limit {
                    file_too_large = Some(FileTooLarge { limit });
                    break;
                }
            }
        }

        buffer = match buffer {
            Buffer::Cursor(mut cursor) => {
                cursor.write_all(bytes.as_ref()).map_err(error::ErrorInternalServerError)?;
                Buffer::Cursor(cursor)
            }
            Buffer::File(mut file) => {
                web::block(move || {
                    let cursor_bytes =
                        opt_cursor.as_ref().map(|x| x.get_ref().as_ref()).unwrap_or_default();

                    file.write_all(cursor_bytes)
                        .and_then(|_| file.write_all(bytes.as_ref()))
                        .map(|_| Buffer::File(file))
                })
                .map_err(error::ErrorInternalServerError)
                .await?
            }
        };
    }

    match (file_too_large, buffer) {
        (Some(FileTooLarge { limit }), _) => {
            Ok((name, Part::File(Err(Error::FileTooLarge { limit, file_name: file_name_opt }))))
        }
        (None, Buffer::Cursor(cursor)) => Ok((name, Part::Text(Bytes::from(cursor.into_inner())))),
        (None, Buffer::File(file)) => {
            Ok((name, Part::File(Ok(File::new(file, file_name_opt, Some(&mime_type))))))
        }
    }
}
