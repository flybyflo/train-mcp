//! End-to-end tests for the Train MCP server.
//!
//! Each test spins up an in-process axum server on a random port, then uses
//! the rmcp Streamable-HTTP client transport to exercise the full MCP protocol:
//! initialize → tools/list → tools/call.

#[cfg(test)]
mod e2e_tests {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use rmcp::model::*;
    use rmcp::service::ServiceExt;
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::tower::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    use rmcp::transport::StreamableHttpClientTransport;
    use serde_json::json;
    use tower_http::cors::CorsLayer;

    use train_mcp::catalog::create_catalog_payload;
    use train_mcp::executor::quickjs::ExecutorLimits;
    use train_mcp::server::TrainMcp;

    // -----------------------------------------------------------------------
    // Test infrastructure
    // -----------------------------------------------------------------------

    fn test_executor_limits() -> ExecutorLimits {
        ExecutorLimits {
            execution_timeout: Duration::from_millis(350),
            queue_timeout: Duration::from_secs(2),
            max_parallel_executions: 8,
            memory_limit_bytes: 16 * 1024 * 1024,
            max_stack_bytes: 256 * 1024,
            gc_threshold_bytes: 4 * 1024 * 1024,
        }
    }

    /// Spin up the MCP server on a random port and return the base URL.
    async fn start_server() -> String {
        start_server_with_options(None, test_executor_limits()).await
    }

    async fn start_server_with_options(
        oebb_base_url: Option<String>,
        executor_limits: ExecutorLimits,
    ) -> String {
        let oebb_base_url =
            oebb_base_url.unwrap_or_else(|| "https://v6.oebb.transport.rest/api".to_string());

        let config = StreamableHttpServerConfig {
            stateful_mode: false,
            ..StreamableHttpServerConfig::default()
        };
        let session_manager = Arc::new(LocalSessionManager::default());
        let oebb_url = oebb_base_url.clone();
        let mcp_service = StreamableHttpService::new(
            move || {
                Ok(TrainMcp::new_with_executor_limits(
                    oebb_url.clone(),
                    executor_limits.clone(),
                ))
            },
            session_manager,
            config,
        );

        let app = Router::new()
            .route("/healthz", get(|| async { Json(json!({"ok": true})) }))
            .route("/catalog", get(|| async { Json(create_catalog_payload()) }))
            .route("/mcp", post(mcp_handler))
            .with_state(mcp_service)
            .layer(CorsLayer::permissive());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{}", addr)
    }

    async fn mcp_handler(
        axum::extract::State(mut service): axum::extract::State<StreamableHttpService<TrainMcp>>,
        request: axum::extract::Request,
    ) -> impl axum::response::IntoResponse {
        use tower_service::Service;
        match service.call(request).await {
            Ok(resp) => resp,
            Err(infallible) => match infallible {},
        }
    }

    /// Connect an rmcp client to the server and return the running service.
    async fn connect_client(base_url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
        let transport = StreamableHttpClientTransport::from_uri(format!("{}/mcp", base_url));
        ().serve(transport)
            .await
            .expect("MCP client initialization failed")
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_initialize_and_list_tools() {
        println!("\n--- [test_initialize_and_list_tools] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        // Server info should be available after initialization.
        let info = client.peer_info().expect("should have peer info");
        println!("Server Info connected: {}", info.server_info.name);
        assert_eq!(info.server_info.name, "train-mcp");

        // List tools.
        println!("Sending list_tools request...");
        let tools_result = peer.list_tools(None).await.expect("list_tools failed");
        let tool_names: Vec<&str> = tools_result.tools.iter().map(|t| t.name.as_ref()).collect();
        println!("Received Tools: {:?}", tool_names);
        assert!(tool_names.contains(&"search"), "should have 'search' tool");
        assert!(
            tool_names.contains(&"execute"),
            "should have 'execute' tool"
        );
        assert_eq!(tools_result.tools.len(), 2, "should have exactly 2 tools");

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_search_list_tools() {
        println!("\n--- [test_search_list_tools] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "const tools = await codemode.listTools({}); return tools;"
        });
        println!(
            "Calling tool 'search' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("search"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool(search) failed");

        assert_eq!(result.is_error, Some(false));

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(structured["ok"], json!(true));
        let tools = &structured["result"];
        assert!(tools.is_array(), "result should be an array of tools");
        let tool_list = tools.as_array().unwrap();
        assert!(!tool_list.is_empty(), "should return at least one tool");
        let names: Vec<&str> = tool_list
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(
            names.contains(&"oebbPlanJourney"),
            "should list oebbPlanJourney"
        );
        assert!(
            names.contains(&"oebbLocations"),
            "should list oebbLocations"
        );
        assert!(names.contains(&"oebbPlanTour"), "should list oebbPlanTour");
        assert!(
            names.contains(&"oebbDepartures"),
            "should list oebbDepartures"
        );
        assert!(names.contains(&"oebbJourneys"), "should list oebbJourneys");
        assert!(names.contains(&"oebbTrip"), "should list oebbTrip");

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_search_get_catalog() {
        println!("\n--- [test_search_get_catalog] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "const catalog = await codemode.getCatalog({}); return catalog;"
        });
        println!(
            "Calling tool 'search' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("search"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool(search) failed");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );
        assert_eq!(structured["ok"], json!(true));
        assert_eq!(result.is_error, Some(false));
        let catalog = &structured["result"];
        assert!(
            catalog["tools"].is_array(),
            "catalog should have tools array"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_empty_code_returns_error() {
        println!("\n--- [test_execute_empty_code_returns_error] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": ""
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should not fail at protocol level");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(
            result.is_error,
            Some(true),
            "empty code should be flagged as tool error"
        );
        assert_eq!(structured["ok"], json!(false), "empty code should fail");
        assert_eq!(structured["error"], json!("execution_error"));
        assert!(
            structured["errorMessage"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("empty"),
            "error should mention empty: {:?}",
            structured["errorMessage"]
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_input_validation_empty_from() {
        println!("\n--- [test_execute_input_validation_empty_from] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return await codemode.oebbPlanJourney({ from: '', to: 'Wien Hbf' });"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should not fail at protocol level");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(
            result.is_error,
            Some(true),
            "invalid input should be flagged as tool error"
        );
        assert_eq!(structured["ok"], json!(false), "invalid input should fail");
        assert_eq!(structured["error"], json!("invalid_input"));
        assert!(
            structured["errorMessage"]
                .as_str()
                .unwrap()
                .contains("from"),
            "errorMessage should explain invalid input"
        );
        let inner = &structured["result"];
        assert_eq!(
            inner["error"],
            json!("invalid_input"),
            "should return invalid_input error"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_input_validation_both_departure_and_arrival() {
        println!("\n--- [test_execute_input_validation_both_departure_and_arrival] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return await codemode.oebbPlanJourney({ from: 'Wien Hbf', to: 'Linz Hbf', departure: '2025-06-01T08:00:00Z', arrival: '2025-06-01T12:00:00Z' });"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should not fail at protocol level");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(
            result.is_error,
            Some(true),
            "invalid input should be flagged as tool error"
        );
        assert_eq!(structured["ok"], json!(false), "invalid input should fail");
        assert_eq!(structured["error"], json!("invalid_input"));
        assert!(
            structured["errorMessage"]
                .as_str()
                .unwrap()
                .contains("departure"),
            "errorMessage should mention departure/arrival conflict"
        );
        let inner = &structured["result"];
        assert_eq!(
            inner["error"],
            json!("invalid_input"),
            "should return invalid_input"
        );
        assert!(
            inner["message"].as_str().unwrap().contains("departure"),
            "message should mention departure/arrival conflict"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_js_rejection_is_flagged_as_error() {
        println!("\n--- [test_execute_js_rejection_is_flagged_as_error] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "throw new Error('boom');"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should not fail at protocol level");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );
        assert_eq!(
            result.is_error,
            Some(true),
            "JS exception should be tool error"
        );
        assert_eq!(structured["ok"], json!(false));
        assert_eq!(structured["error"], json!("execution_error"));
        assert!(
            structured["errorMessage"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("rejected"),
            "errorMessage should indicate promise rejection"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_busy_loop_times_out() {
        println!("\n--- [test_execute_busy_loop_times_out] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "while (true) {}"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should return timeout error payload");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );
        assert_eq!(
            result.is_error,
            Some(true),
            "timeout must be marked as error"
        );
        assert_eq!(structured["ok"], json!(false));
        assert_eq!(structured["error"], json!("execution_error"));
        assert!(
            structured["errorMessage"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("timed out")
                || structured["errorMessage"]
                    .as_str()
                    .unwrap()
                    .to_lowercase()
                    .contains("interrupt"),
            "errorMessage should indicate timeout/interrupt"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_plain_error_field_is_not_auto_error() {
        println!("\n--- [test_execute_plain_error_field_is_not_auto_error] ---");
        let url = start_server().await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return { error: 'data_field_only', message: 'not a tool error', value: 123 };"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should succeed");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );
        assert_eq!(result.is_error, Some(false));
        assert_eq!(structured["ok"], json!(true));
        assert_eq!(structured["error"], serde_json::Value::Null);
        assert_eq!(structured["result"]["error"], json!("data_field_only"));
        assert_eq!(structured["result"]["value"], json!(123));

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_upstream_failure_is_classified() {
        println!("\n--- [test_execute_upstream_failure_is_classified] ---");
        let mut limits = test_executor_limits();
        limits.execution_timeout = Duration::from_secs(3);
        let url = start_server_with_options(Some("http://127.0.0.1:9".to_string()), limits).await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return await codemode.oebbLocations({ query: 'Wien', results: 2 });"
        });
        println!(
            "Calling tool 'execute' with args:\n{}",
            serde_json::to_string_pretty(&args).unwrap()
        );

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool should not fail at protocol level");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );
        assert_eq!(result.is_error, Some(true));
        assert_eq!(structured["ok"], json!(false));
        assert_eq!(structured["error"], json!("oebbLocations_failed"));
        assert!(
            structured["errorMessage"].as_str().unwrap().len() > 8,
            "should expose upstream transport failure context"
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_departures_resolves_station_name_and_limit_alias() {
        println!("\n--- [test_execute_departures_resolves_station_name_and_limit_alias] ---");

        let mock_app = Router::new()
            .route(
                "/locations",
                get(|| async {
                    Json(json!([
                        {
                            "id": "1230501",
                            "name": "Amstetten NÖ Bahnhof",
                            "type": "stop"
                        }
                    ]))
                }),
            )
            .route(
                "/stops/1230501/departures",
                get(|| async {
                    Json(json!({
                        "departures": [
                            {
                                "when": "2026-02-26T01:29:00+01:00",
                                "line": { "name": "RJ 820", "mode": "train" },
                                "direction": "Wels Hbf"
                            }
                        ]
                    }))
                }),
            );

        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let mock_addr = mock_listener.local_addr().expect("mock local addr");
        tokio::spawn(async move {
            axum::serve(mock_listener, mock_app)
                .await
                .expect("mock server failed");
        });

        let base = format!("http://{}", mock_addr);
        let url = start_server_with_options(Some(base), test_executor_limits()).await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return await codemode.oebbDepartures({ station: 'Amstetten NÖ Bahnhof', limit: 1 });"
        });

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool(execute) failed");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(result.is_error, Some(false));
        assert_eq!(structured["ok"], json!(true));
        assert_eq!(
            structured["result"]["departures"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            structured["result"]["departures"][0]["line"]["name"],
            json!("RJ 820")
        );

        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn test_execute_plan_tour_chains_legs_with_waits() {
        println!("\n--- [test_execute_plan_tour_chains_legs_with_waits] ---");

        let mock_app = Router::new()
            .route(
                "/locations",
                get(|axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
                    let query = q.get("query").map(String::as_str).unwrap_or("");
                    let payload = match query {
                        "Amstetten NÖ Bahnhof" => json!([{ "id": "8100012", "name": "Amstetten NÖ Bahnhof", "type": "stop" }]),
                        "Salzburg Hbf" => json!([{ "id": "8100002", "name": "Salzburg Hbf", "type": "stop" }]),
                        "Klagenfurt Hbf" => json!([{ "id": "8100085", "name": "Klagenfurt Hbf", "type": "stop" }]),
                        _ => json!([]),
                    };
                    Json(payload)
                }),
            )
            .route(
                "/journeys",
                get(|axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
                    let from = q.get("from").map(String::as_str).unwrap_or("");
                    let to = q.get("to").map(String::as_str).unwrap_or("");
                    let payload = match (from, to) {
                        ("8100012", "8100002") => json!({
                            "journeys": [{
                                "departure": "2026-02-27T08:00:00+01:00",
                                "arrival": "2026-02-27T10:00:00+01:00",
                                "legs": [{
                                    "departure": "2026-02-27T08:00:00+01:00",
                                    "arrival": "2026-02-27T10:00:00+01:00",
                                    "origin": { "id": "8100012", "name": "Amstetten NÖ Bahnhof" },
                                    "destination": { "id": "8100002", "name": "Salzburg Hbf" },
                                    "line": { "name": "IC 100", "mode": "train", "product": "national", "operator": { "id": "oebb", "name": "ÖBB" } }
                                }]
                            }]
                        }),
                        ("8100002", "8100085") => json!({
                            "journeys": [{
                                "departure": "2026-02-27T11:30:00+01:00",
                                "arrival": "2026-02-27T13:00:00+01:00",
                                "legs": [{
                                    "departure": "2026-02-27T11:30:00+01:00",
                                    "arrival": "2026-02-27T13:00:00+01:00",
                                    "origin": { "id": "8100002", "name": "Salzburg Hbf" },
                                    "destination": { "id": "8100085", "name": "Klagenfurt Hbf" },
                                    "line": { "name": "RJ 200", "mode": "train", "product": "national", "operator": { "id": "oebb", "name": "ÖBB" } }
                                }]
                            }]
                        }),
                        _ => json!({ "journeys": [] }),
                    };
                    Json(payload)
                }),
            );

        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let mock_addr = mock_listener.local_addr().expect("mock local addr");
        tokio::spawn(async move {
            axum::serve(mock_listener, mock_app)
                .await
                .expect("mock server failed");
        });

        let base = format!("http://{}", mock_addr);
        let url = start_server_with_options(Some(base), test_executor_limits()).await;
        let client = connect_client(&url).await;
        let peer = client.peer();

        let args = json!({
            "code": "return await codemode.oebbPlanTour({ departure: '2026-02-27T08:00:00+01:00', selection: 'earliest_arrival', legs: [ { from: 'Amstetten NÖ Bahnhof', to: 'Salzburg Hbf', minStopMinutesAfter: 90 }, { from: 'Salzburg Hbf', to: 'Klagenfurt Hbf' } ] });"
        });

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed("execute"),
                arguments: Some(serde_json::from_value(args).unwrap()),
                task: None,
            })
            .await
            .expect("call_tool(execute) failed");

        let structured = result
            .structured_content
            .expect("should have structuredContent");

        println!(
            "Tool Result:\n{}",
            serde_json::to_string_pretty(&structured).unwrap()
        );

        assert_eq!(result.is_error, Some(false));
        assert_eq!(structured["ok"], json!(true));
        assert_eq!(
            structured["result"]["selectedJourney"]["departure"],
            json!("2026-02-27T08:00:00+01:00")
        );
        assert_eq!(
            structured["result"]["selectedJourney"]["arrival"],
            json!("2026-02-27T13:00:00+01:00")
        );
        assert_eq!(
            structured["result"]["selectedJourney"]["plannedStopovers"][0]["waitMinutes"],
            json!(90)
        );

        let _ = client.cancel().await;
    }
}
