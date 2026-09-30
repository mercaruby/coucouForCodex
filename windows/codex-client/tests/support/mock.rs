//! Protocol fixture: no network, credentials or inference.
use serde_json::{json, Value};
use std::io::{self, BufRead};
fn emit(value: Value) {
    println!("{value}");
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|s| s == "generate-json-schema") {
        let out = std::path::Path::new(args.last().unwrap());
        let mut schema = json!({"oneOf":[
            {"properties":{"type":{"enum":["readOnly"]},"access":{}}},
            {"properties":{"type":{"enum":["restricted"]},"readableRoots":{},"includePlatformDefaults":{}}}
        ]});
        if std::env::current_exe().unwrap().file_stem().unwrap() == "legacy-codex" {
            schema = json!({"properties":{"type":{"enum":["readOnly"]},"networkAccess":{"type":"boolean"}}});
        }
        std::fs::write(out.join("ClientRequest.json"), schema.to_string()).unwrap();
        return;
    }
    let api_mode = std::env::current_dir().unwrap().join("api-mode").exists();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = message["id"].clone();
        match message["method"].as_str().unwrap_or("") {
            "initialize" => emit(json!({"id":id,"result":{}})),
            "initialized" => {}
            "account/read" => emit(
                json!({"id":id,"result":{"account":{"type":if api_mode {"apiKey"} else {"chatgpt"}}}}),
            ),
            "thread/start" => {
                assert_eq!(message["params"]["sandbox"], "readOnly");
                assert_eq!(message["params"]["approvalPolicy"], "never");
                assert_eq!(message["params"]["ephemeral"], true);
                emit(json!({"id":id,"result":{"thread":{"id":"fixture"}}}));
            }
            "turn/start" => {
                assert_eq!(
                    message["params"]["sandboxPolicy"]["access"]["type"],
                    "restricted"
                );
                assert_eq!(
                    message["params"]["sandboxPolicy"]["access"]["includePlatformDefaults"],
                    false
                );
                emit(json!({"id":id,"result":{"turn":{"id":"turn"}}}));
                emit(
                    json!({"method":"item/completed","params":{"threadId":"fixture","item":{"type":"agentMessage","text":"Fixture reply"}}}),
                );
                emit(
                    json!({"id":"approval-fixture","method":"item/commandExecution/requestApproval","params":{}}),
                );
            }
            _ => {
                if id == "approval-fixture" {
                    assert!(
                        message.get("error").is_some(),
                        "must refuse automatic approval"
                    );
                    emit(
                        json!({"method":"turn/completed","params":{"threadId":"fixture","turn":{"status":"completed"}}}),
                    );
                }
            }
        }
    }
}
