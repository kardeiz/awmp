use super::*;
use actix_multipart_v01::{Field, Multipart};
use actix_web_v1::{dev, error, http, web, Error as ActixWebError, FromRequest, HttpRequest};
use futures_v01::{
    future::{self, Either},
    Future, IntoFuture, Stream,
};

impl FromRequest for Parts {
    type Error = ActixWebError;
    type Future = Box<dyn Future<Item = Self, Error = Self::Error>>;
    type Config = PartsConfig;

    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let opt_cfg = req.get_app_data::<PartsConfig>();

        let rt = Multipart::from_request(req, payload)
            .into_future()
            .map(|mp| mp.map_err(error::ErrorInternalServerError))
            .flatten_stream()
            .map(move |field| handle_field(opt_cfg.clone(), field).into_stream())
            .flatten()
            .collect()
            .map(|parts| {
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
            });

        Box::new(rt)
    }
}

fn new_temp_file(
    opt_cfg: Option<web::Data<PartsConfig>>,
) -> impl Future<Item = NamedTempFile, Error = error::Error> {
    web::block(move || match opt_cfg.as_ref().and_then(|x| x.temp_dir.as_ref()) {
        Some(temp_dir) => NamedTempFile::new_in(temp_dir),
        _ => NamedTempFile::new(),
    })
    .map_err(error::ErrorInternalServerError)
}

fn handle_field(
    opt_cfg: Option<web::Data<PartsConfig>>,
    field: Field,
) -> impl Future<Item = (String, Part), Error = error::Error> {
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
            return Either::A(future::err(error::ErrorInternalServerError(
                "Field name is required",
            )));
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

    let buffer_fut = match file_name_opt.as_ref() {
        Some(_) if !marked_as_text => Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File)),
        None if marked_as_file => Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File)),
        _ => Either::B(future::ok(Buffer::Cursor(Cursor::new(Vec::new())))),
    };

    let rt =
        future::loop_fn(Either::A(buffer_fut.map(|buffer| (field, buffer, 0))), move |state| {
            let opt_cfg = opt_cfg.clone();
            state.and_then(move |(stream, buffer, mut len)| {
                let opt_cfg = opt_cfg.clone();
                stream.into_future().map_err(|(e, _)| error::ErrorInternalServerError(e)).and_then(
                    move |(bytes, new_stream)| match bytes {
                        Some(bytes) => {
                            let opt_cfg = opt_cfg.clone();

                            len += bytes.len();

                            let mut opt_cursor = None;

                            let buffer_fut = if opt_cfg
                                .as_ref()
                                .and_then(|x| x.text_limit)
                                .map(|x| len > x)
                                .unwrap_or(false)
                            {
                                match buffer {
                                    Buffer::Cursor(cursor) => {
                                        opt_cursor = Some(cursor);
                                        Either::A(new_temp_file(opt_cfg.clone()).map(Buffer::File))
                                    }
                                    x => Either::B(future::ok(x)),
                                }
                            } else {
                                Either::B(future::ok(buffer))
                            };

                            Either::A(buffer_fut.map(move |buffer| match buffer {
                                Buffer::Cursor(mut cursor) => {
                                    if let Err(e) = cursor.write_all(bytes.as_ref()) {
                                        return future::Loop::Break(future::err(
                                            error::ErrorInternalServerError(e),
                                        ));
                                    }
                                    future::Loop::Continue(Either::B(Either::A(future::ok((
                                        new_stream,
                                        Buffer::Cursor(cursor),
                                        len,
                                    )))))
                                }
                                Buffer::File(mut file) => {
                                    if let Some(limit) = opt_cfg.as_ref().and_then(|x| x.file_limit)
                                    {
                                        if len > limit {
                                            return future::Loop::Break(future::ok(Either::A(
                                                FileTooLarge { limit },
                                            )));
                                        }
                                    }

                                    let rt = web::block(move || {
                                        let cursor_bytes = opt_cursor
                                            .as_ref()
                                            .map(|x| x.get_ref().as_ref())
                                            .unwrap_or_default();

                                        file.write_all(cursor_bytes)
                                            .and_then(|_| file.write_all(bytes.as_ref()))
                                            .map(|_| Buffer::File(file))
                                    })
                                    .map(move |buffer| (new_stream, buffer, len))
                                    .map_err(error::ErrorInternalServerError);
                                    future::Loop::Continue(Either::B(Either::B(rt)))
                                }
                            }))
                        }
                        None => Either::B(future::ok(future::Loop::Break(future::ok(Either::B(
                            buffer,
                        ))))),
                    },
                )
            })
        })
        .flatten()
        .map(move |buffer| match buffer {
            Either::B(Buffer::Cursor(cursor)) => {
                (name, Part::Text(Bytes::from(cursor.into_inner())))
            }
            Either::B(Buffer::File(file)) => {
                let sanitized_file_name = match file_name_opt {
                    Some(ref s) => sanitize_filename::sanitize(s),
                    None => {
                        let uuid = uuid::Uuid::new_v4().to_simple();
                        match mime_guess::get_mime_extensions(&mime_type).and_then(|x| x.first()) {
                            Some(ext) => format!("{}.{}", uuid, ext),
                            None => uuid.to_string(),
                        }
                    }
                };
                (
                    name,
                    Part::File(Ok(File {
                        inner: file,
                        sanitized_file_name,
                        original_file_name: file_name_opt,
                    })),
                )
            }
            Either::A(FileTooLarge { limit }) => {
                (name, Part::File(Err(Error::FileTooLarge { limit, file_name: file_name_opt })))
            }
        });

    Either::B(rt)
}
