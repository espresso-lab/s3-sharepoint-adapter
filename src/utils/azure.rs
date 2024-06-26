use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, errors::Error as JwtError, Algorithm, DecodingKey, Validation};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, Error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info};

use crate::config;

#[derive(Debug, Clone)]
struct TokenData {
    access_token: String,
    expires_at: DateTime<Utc>,
}

static TOKEN_DATA: Lazy<Arc<AsyncMutex<Option<TokenData>>>> =
    Lazy::new(|| Arc::new(AsyncMutex::new(None)));

#[derive(Deserialize, Debug)]
pub struct SearchRequest {
    pub query: String,
    pub prefix: String,
    pub max_keys: Option<u16>,
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize, Debug)]
pub struct GetAzureObjectResponse {
    pub content_type: String,
    pub data: Vec<u8>,
    pub file_name: String,
}

#[derive(Deserialize, Debug)]
pub struct HeadAzureObjectResponse {
    pub content_type: String,
    pub status_code: u16,
    pub size: u64,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SharePointObjects {
    #[serde(rename = "value")]
    pub items: Vec<Item>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Item {
    #[serde(rename = "createdDateTime")]
    pub created_date_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "eTag")]
    pub e_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "lastModifiedDateTime")]
    pub last_modified_date_time: Option<String>,
    pub name: String,
    #[serde(rename = "webUrl")]
    pub web_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<Folder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<File>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Folder {
    #[serde(rename = "childCount")]
    pub child_count: u32,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct File {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

#[derive(Debug, Deserialize)]
struct Claims {
    exp: i64,
}

fn prepare_prefix(prefix: String, search_query: String) -> String {
    if prefix == "/" || prefix.is_empty() {
        if search_query.is_empty() {
            format!("/children")
        } else {
            format!("/search(q='{}')", search_query)
        }
    } else {
        if search_query.is_empty() {
            format!(
                ":/{}:/children",
                prefix.trim_start_matches("/").trim_end_matches("/")
            )
        } else {
            format!(
                ":/{}:/search(q='{}')",
                prefix.trim_start_matches("/").trim_end_matches("/"),
                search_query
            )
        }
    }
}

fn decode_no_verify(token: &str) -> Result<DateTime<Utc>, JwtError> {
    let mut no_verify = Validation::new(Algorithm::RS256);
    no_verify.insecure_disable_signature_validation();
    no_verify.set_audience(&["https://graph.microsoft.com".to_string()]);
    match decode::<Claims>(
        token,
        &DecodingKey::from_secret("noverify".as_bytes()),
        &no_verify,
    ) {
        Ok(token_data) => Ok(DateTime::from_timestamp(token_data.claims.exp, 0).unwrap()),
        Err(err) => Err(err),
    }
}

async fn fetch_token() -> Result<TokenData, Error> {
    let tenant = config().tenant.clone();
    let client_id = config().app_client_id.clone();
    let client_secret = config().app_client_secret.clone();

    let url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant
    );

    let client = Client::new();
    match client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", client_id),
            ("scope", "https://graph.microsoft.com/.default".to_owned()),
            ("client_secret", client_secret),
            ("grant_type", "client_credentials".to_owned()),
        ])
        .send()
        .await
        .unwrap()
        .json::<TokenResponse>()
        .await
    {
        Ok(response) => Ok(TokenData {
            access_token: response.access_token.clone(),
            expires_at: decode_no_verify(&response.access_token).unwrap(),
        }),
        Err(err) => Err(err),
    }
}

async fn get_token() -> Result<String, Error> {
    let token_data = TOKEN_DATA.lock().await;
    if let Some(ref data) = *token_data {
        if data.expires_at > Utc::now() {
            info!(
                "Token is still valid until: {} - UTC Now: {}",
                data.expires_at,
                Utc::now()
            );
            return Ok(data.access_token.clone());
        }
    }
    drop(token_data); // Explicitly drop to release the lock before fetching new token
    let new_token_data = fetch_token().await.unwrap();

    let mut token_data = TOKEN_DATA.lock().await;
    *token_data = Some(new_token_data.clone());
    debug!("New token fetched and stored");

    Ok(new_token_data.access_token)
}

pub async fn list_azure_objects(
    site_id: String,
    prefix: String,
    max_keys: u16,
    search_query: Option<String>,
) -> Result<SharePointObjects, Error> {
    let search_query = search_query.unwrap_or("".to_string());
    match get_token().await {
        Ok(token) => {
            let relative_path = prepare_prefix(prefix, search_query.clone());
            let url = format!(
                "https://graph.microsoft.com/v1.0/sites/{}/drive/root{}?$top={}",
                site_id, relative_path, max_keys
            );
            let client = Client::new();
            match client
                .get(url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
                .unwrap()
                .json::<SharePointObjects>()
                .await
            {
                Ok(objects) => Ok(objects),
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

pub async fn head_azure_object(
    site_id: String,
    file_path: String,
) -> Result<HeadAzureObjectResponse, Error> {
    let filename_pattern = config().filename_pattern.clone();
    let regex = Regex::new(&filename_pattern).unwrap();
    let part = if file_path.is_empty() || file_path.eq("/") {
        ""
    } else {
        ":/"
    };
    let key = if file_path.is_empty() {
        "/".to_string()
    } else {
        file_path.clone()
    };
    match get_token().await {
        Ok(token) => {
            let url = format!(
                "https://graph.microsoft.com/v1.0/sites/{}/drive/root{}{}",
                site_id, part, key
            );
            let client = Client::new();
            match client
                .get(url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
                .unwrap()
                .json::<Item>()
                .await
            {
                Ok(result) => {
                    if key.ends_with('/') {
                        if result.folder.is_some() {
                            Ok(HeadAzureObjectResponse {
                                content_type: "application/xml".to_string(),
                                status_code: 200,
                                size: 0,
                            })
                        } else {
                            Ok(HeadAzureObjectResponse {
                                content_type: "application/xml".to_string(),
                                status_code: 404,
                                size: 0,
                            })
                        }
                    } else {
                        if result.file.is_some() {
                            if !regex.is_match(&result.name) {
                                return Ok(HeadAzureObjectResponse {
                                    content_type: "application/xml".to_string(),
                                    status_code: 403,
                                    size: 0,
                                });
                            }
                            Ok(HeadAzureObjectResponse {
                                content_type: result.file.unwrap().mime_type,
                                status_code: 200,
                                size: result.size.unwrap_or(0),
                            })
                        } else {
                            Ok(HeadAzureObjectResponse {
                                content_type: "application/xml".to_string(),
                                status_code: 404,
                                size: 0,
                            })
                        }
                    }
                }
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

pub async fn get_azure_object_data(
    site_id: String,
    file_path: String,
) -> Result<GetAzureObjectResponse, Error> {
    match get_token().await {
        Ok(token) => {
            let url = format!(
                "https://graph.microsoft.com/v1.0/sites/{}/drive/root:/{}:/content",
                site_id, file_path
            );
            let file_name = file_path.split('/').last().unwrap_or_default();
            let client = Client::new();
            match client
                .get(url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(objects) => Ok(GetAzureObjectResponse {
                    content_type: objects
                        .headers()
                        .get("Content-Type")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                    data: objects.bytes().await.unwrap().to_vec(),
                    file_name: file_name.to_string(),
                }),
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}
