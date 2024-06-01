mod utils;

use std::env;

use dotenv::dotenv;
use salvo::http::StatusCode;
use salvo::prelude::*;
use utils::azure::{get_azure_object_data, head_azure_object, list_azure_objects};
use utils::s3::generate_s3_list_objects_v2_response;

#[handler]
async fn ok_handler(res: &mut Response) {
    res.status_code(StatusCode::OK).render(Text::Plain("OK"))
}

#[handler]
async fn bad_request_handler(res: &mut Response) {
    res.status_code(StatusCode::BAD_REQUEST)
        .render(Text::Plain("BAD REQUEST"))
}

#[handler]
async fn head_handler(req: &mut Request, res: &mut Response) {
    let site_id = env::var("SHAREPOINT_SITE_ID").expect("SHAREPOINT_SITE_ID not found");
    let key = req.params().get("**path").cloned().unwrap_or_default();
    match head_azure_object(site_id.clone(), key.clone()).await {
        Ok(result) => {
            res.headers_mut()
                .insert("Content-Type", result.content_type.parse().unwrap());
            res.headers_mut()
                .insert("Content-Length", result.size.to_string().parse().unwrap());
            res.status_code(StatusCode::from_u16(result.status_code).unwrap());
        }
        Err(_) => {
            res.headers_mut()
                .insert("Content-Type", "application/xml".parse().unwrap());
            res.headers_mut()
                .insert("Content-Length", "0".parse().unwrap());
            res.status_code(StatusCode::NOT_FOUND);
        }
    }
}

#[handler]
async fn list_objects_v1(req: &mut Request, res: &mut Response) {
    let prefix = req
        .query::<String>("prefix")
        .unwrap_or("/".to_string())
        .trim_end_matches("/")
        .to_string();
    let max_keys = req.query::<i16>("max-keys").unwrap_or(1000);
    let site_id = env::var("SHAREPOINT_SITE_ID").expect("SHAREPOINT_SITE_ID not found");
    match list_azure_objects(site_id.clone(), prefix.clone(), max_keys, None).await {
        Ok(objects) => {
            res.status_code(StatusCode::OK).render(Text::Xml(
                generate_s3_list_objects_v2_response(site_id, prefix, objects, false),
            ));
        }
        Err(err) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[handler]
async fn get_object(req: &mut Request, res: &mut Response) {
    let site_id = env::var("SHAREPOINT_SITE_ID").expect("SHAREPOINT_SITE_ID not found");
    let key = req.params().get("**path").cloned().unwrap_or_default();
    match get_azure_object_data(site_id.clone(), key.clone()).await {
        Ok(result) => {
            res.headers_mut()
                .insert("Content-Type", result.content_type.parse().unwrap());
            res.headers_mut().insert(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", result.file_name)
                    .parse()
                    .unwrap(),
            );
            let _ = res.write_body(result.data);
        }
        Err(err) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt().init();

    let router = Router::new()
        .push(Router::with_path("<**path>").head(head_handler))
        .push(Router::with_path("status").get(ok_handler))
        .push(
            Router::with_filter_fn(|req, _| {
                req.query::<i8>("list-type").is_none()
                    && req.query::<String>("prefix").is_some()
                    && (req.query::<String>("delimiter").is_some()
                        || req.query::<String>("max-keys").is_some())
            })
            .get(list_objects_v1),
        )
        .push(Router::with_path("<**path>").get(get_object))
        .goal(bad_request_handler);
    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new("0.0.0.0:3000").bind().await;
    Server::new(acceptor).serve(service).await;
}
