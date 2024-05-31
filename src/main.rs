mod utils;

use dotenv::dotenv;
use salvo::http::StatusCode;
use salvo::prelude::*;
use tracing::info;
use utils::azure::{get_azure_object, list_azure_objects};
use utils::s3::{generate_s3_list_objects_v2_response, GetObjectRequest, ListObjectsV2Request};

#[handler]
async fn ok_handler(res: &mut Response) {
    res.status_code(StatusCode::OK).render(Text::Plain("OK"))
}

#[handler]
async fn list_objects_v2(req: &mut Request, res: &mut Response) {
    let payload = req.parse_json::<ListObjectsV2Request>().await.unwrap();
    let files_only = payload
        .search_query
        .to_owned()
        .unwrap_or("".to_string())
        .is_empty();
    match list_azure_objects(
        payload.bucket.clone(),
        payload.prefix.clone().unwrap_or("/".to_string()),
        payload.search_query,
    )
    .await
    {
        Ok(objects) => {
            res.status_code(StatusCode::OK).render(Text::Xml(
                generate_s3_list_objects_v2_response(
                    payload.bucket.clone(),
                    payload.prefix.clone().unwrap_or("/".to_string()),
                    objects,
                    !files_only,
                ),
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
    let payload = req.parse_json::<GetObjectRequest>().await.unwrap();
    match get_azure_object(payload.bucket.clone(), payload.key.clone()).await {
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

#[handler]
async fn test(_req: &mut Request, _res: &mut Response) {
    info!("test");
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt().init();

    let router = Router::new()
        .hoop(test)
        .push(Router::with_path("status").get(ok_handler))
        .push(Router::with_path("/listObjectsV2").post(list_objects_v2))
        .push(Router::with_path("/getObject").post(get_object))
        .goal(ok_handler);
    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new("0.0.0.0:3000").bind().await;
    Server::new(acceptor).serve(service).await;
}
