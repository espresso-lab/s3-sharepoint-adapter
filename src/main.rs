mod utils;

use std::env;

use dotenv::dotenv;
use salvo::http::StatusCode;
use salvo::prelude::*;
use utils::azure::{get_azure_object, list_azure_objects};
use utils::s3::{generate_s3_list_objects_v2_response, GetObjectRequest, ListObjectsV2Request};

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
            // print object json
            // print!("{}", serde_json::to_string(&objects).unwrap());
            res.status_code(StatusCode::OK).render(Text::Xml(
                generate_s3_list_objects_v2_response(site_id, prefix, objects, false),
            ));
        }
        Err(err) => {
            print!("{}", err.to_string());
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[handler]
async fn get_object(req: &mut Request, res: &mut Response) {
    let site_id = env::var("SHAREPOINT_SITE_ID").expect("SHAREPOINT_SITE_ID not found");
    let key = req.params().get("**path").cloned().unwrap_or_default();
    match get_azure_object(site_id.clone(), key.clone()).await {
        Ok(result) => {
            res.headers_mut()
                .insert("Content-Type", result.content_type.parse().unwrap());
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
        .push(Router::with_path("status").get(ok_handler))
        .push(
            Router::with_filter_fn(|req, _| req.query::<i8>("list-type").unwrap_or(0).eq(&2))
                .get(list_objects_v1),
        )
        // .push(
        //     Router::with_filter_fn(|req, _| {
        //         req.query::<i8>("list-type").is_none()
        //             && req.query::<i8>("prefix").is_some()
        //             && req.query::<i8>("delimiter").is_some()
        //     })
        //     .get(list_objects_v2),
        // )
        .push(Router::with_path("<**path>").get(get_object))
        .goal(bad_request_handler);
    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new("0.0.0.0:3000").bind().await;
    Server::new(acceptor).serve(service).await;
}
