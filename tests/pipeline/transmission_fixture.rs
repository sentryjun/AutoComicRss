// SPDX-License-Identifier: GPL-2.0-only
// Shared wire fixture for the pipeline business contracts.
use super::*;
pub(super) async fn configured_service(
    store: Store,
    settings: &Config,
) -> autocomic_rss::error::Result<Service> {
    let transmission = settings
        .qbit
        .as_ref()
        .filter(|c| c.url.ends_with("/rpc"))
        .cloned();
    let mut settings = settings.clone();
    if transmission.is_some() {
        settings.qbit = None;
    }
    let service = Service::new(store, &settings).await?;
    if let Some(connection) = transmission {
        if service.store.downloaders().await?.is_empty() {
            let instance = service
                .store
                .create_downloader(autocomic_rss::domain::DownloaderInput {
                    name: "Transmission".into(),
                    kind: autocomic_rss::domain::DownloaderKind::Transmission,
                    connection,
                })
                .await?;
            service.store.activate_downloader(&instance.id).await?;
        }
    }
    Ok(service)
}

pub(super) async fn transmission_rpc(
    req: HttpRequest,
    body: web::Json<Value>,
    state: web::Data<Arc<Mutex<Remote>>>,
) -> HttpResponse {
    let mut remote = state.lock().unwrap();
    let auth = req
        .headers()
        .get("Authorization")
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("Basic ")
        .unwrap();
    let decoded =
        String::from_utf8(data_encoding::BASE64.decode(auth.as_bytes()).unwrap()).unwrap();
    assert!(decoded == "test:test" || decoded == "test:B-private-password");
    if remote.rpc_auth_failure {
        return HttpResponse::Unauthorized().body("private-server-detail");
    }
    let is_add = matches!(body["method"].as_str(), Some("torrent_add" | "torrent-add"));
    if is_add && remote.rpc_rotate_on_add && !remote.rpc_rotated {
        remote.rpc_rotated = true;
    }
    let session_token = if remote.rpc_rotated {
        "rotated-session"
    } else {
        "fixture-session"
    };
    if remote.rpc_always_conflict
        || req
            .headers()
            .get("X-Transmission-Session-Id")
            .is_none_or(|v| v != session_token)
    {
        remote.rpc_conflicts += 1;
        return HttpResponse::Conflict()
            .insert_header(("X-Transmission-Session-Id", session_token))
            .finish();
    }
    let method = body["method"].as_str().unwrap();
    remote.rpc_requests.push(method.into());
    if !remote.rpc_modern && method == "session_get" {
        return HttpResponse::Ok()
            .json(json!({"result":"method name not recognized","arguments":{}}));
    }
    let modern = remote.rpc_modern;
    if remote.rpc_fail_method.as_deref() == Some(method) {
        return HttpResponse::Ok().json(if modern {
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"private-server-detail"}})
        } else { json!({"tag":1,"result":"private-server-detail","arguments":{}}) });
    }
    let key = |new: &'static str, old: &'static str| if modern { new } else { old };
    let params = &body[key("params", "arguments")];
    if modern {
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 1);
    } else {
        assert_eq!(body["tag"], 1);
    }
    let result = match method {
        "session_get" | "session-get" => json!({"version":if modern { "4.1.0" } else { "3.00" }}),
        "torrent_get" | "torrent-get" => {
            if remote.fail_list {
                return HttpResponse::ServiceUnavailable().finish();
            }
            let fields = params["fields"].as_array().unwrap();
            let mut rows = Vec::new();
            for torrent in &remote.torrents {
                if let Some(ids) = params["ids"].as_array() {
                    assert!(ids.iter().all(Value::is_string));
                    if !ids.contains(&torrent["hash"]) {
                        continue;
                    }
                }
                let mut row = json!({});
                for field in fields {
                    let field = field.as_str().unwrap();
                    row[field] = match field {
                        "id" => json!(remote.rpc_id),
                        "hash_string" | "hashString" => torrent["hash"].clone(),
                        "download_dir" | "downloadDir" => torrent["save_path"].clone(),
                        "percent_done" | "percentDone" => torrent["progress"].clone(),
                        "metadata_percent_complete" | "metadataPercentComplete" => {
                            torrent.get("metadata").cloned().unwrap_or(json!(1.0))
                        }
                        "status" => {
                            torrent
                                .get("status")
                                .cloned()
                                .unwrap_or(json!(if torrent["progress"].as_f64().unwrap() > 0.0 {
                                    4
                                } else {
                                    0
                                }))
                        }
                        "error" => torrent.get("error").cloned().unwrap_or(json!(0)),
                        "files" | "wanted" => {
                            let files = remote.files.clone().unwrap_or(vec![
                                json!({"name":"Series/ep03.mkv","size":3,"priority":1}),
                                json!({"name":"skipped.mkv","size":10,"priority":0}),
                            ]);
                            if field == "files" {
                                json!(files
                                    .iter()
                                    .map(|f| json!({"name":f["name"],"length":f["size"]}))
                                    .collect::<Vec<_>>())
                            } else {
                                json!(files
                                    .iter()
                                    .map(|f| {
                                        let wanted = f["priority"] != 0;
                                        if modern {
                                            json!(wanted)
                                        } else {
                                            json!(i32::from(wanted))
                                        }
                                    })
                                    .collect::<Vec<_>>())
                            }
                        }
                        _ => panic!("unexpected RPC field {field}"),
                    };
                }
                rows.push(row);
            }
            json!({"torrents":rows})
        }
        "torrent_add" | "torrent-add" => {
            let hash = if let Some(encoded) = params["metainfo"].as_str() {
                assert!(params.get("filename").is_none());
                let bytes = data_encoding::BASE64.decode(encoded.as_bytes()).unwrap();
                assert_eq!(bytes, TORRENT);
                autocomic_rss::downloaders::torrent::torrent_hash(&bytes).unwrap()
            } else {
                autocomic_rss::downloaders::torrent::magnet_hash(
                    params["filename"].as_str().unwrap(),
                )
                .unwrap()
            };
            assert_eq!(params["paused"], false);
            let path = params[key("download_dir", "download-dir")]
                .as_str()
                .unwrap()
                .to_owned();
            remote.adds += 1;
            remote.save_paths.push(path.clone());
            let duplicate_path = remote.rpc_duplicate_path.clone();
            remote.torrents.push(json!({"hash":hash,"progress":0.0,"save_path":duplicate_path.clone().unwrap_or(path)}));
            if remote.fail_after_add {
                remote.fail_after_add = false;
                return HttpResponse::InternalServerError().finish();
            }
            let result_key = if duplicate_path.is_some() {
                key("torrent_duplicate", "torrent-duplicate")
            } else {
                key("torrent_added", "torrent-added")
            };
            json!({result_key: {"id":remote.rpc_id,"name":"test",key("hash_string","hashString"):hash}})
        }
        _ => panic!("unexpected RPC method {method}"),
    };
    HttpResponse::Ok().json(if modern {
        json!({"jsonrpc":"2.0","id":1,"result":result})
    } else {
        json!({"tag":1,"result":"success","arguments":result})
    })
}
