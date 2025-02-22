use ark_bn254::g1::G1Affine;
use eigen_services_avsregistry::AvsRegistryService;
use eigen_services_operatorsinfo::operator_info::OperatorInfoService;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregatorService, TaskMetadata, TaskSignature};
use eigen_client_avsregistry::reader::{AvsRegistryChainReader, AvsRegistryReader};
use eigen_logging::{get_logger, init_logger};
use alloy_provider::{Provider,RootProvider};
use eigen_logging::logger::Logger;
use eigen_logging::noop_logger::NoopLogger;
use eigen_services_avsregistry::chaincaller::AvsRegistryServiceChainCaller;
use eigen_services_operatorsinfo::operatorsinfo_inmemory::OperatorInfoServiceInMemory;
use eigen_crypto_bls::Signature;
use eigen_types::{
    avs::{TaskIndex, TaskResponseDigest, SignedTaskResponseDigest},
    operator::QuorumThresholdPercentages,
};
use alloy_primitives::{Address, Bytes, FixedBytes, address};
use alloy_network::Ethereum;
use url::Url;
use std::time::Duration;
use std::str::FromStr;
use std::sync::Arc;
use ark_ec::AffineRepr;
use ark_ff::PrimeField;
use std::env;
use dotenv::dotenv;

use axum::{
    routing::post,
    Json,
    Router,
};
use tracing::{info, debug, error};
use tokio_util::sync::CancellationToken;
use tokio::{task, time::sleep};

fn parse_signature(sig: &str) -> G1Affine {
    // Remove parentheses and split by comma
    let sig = sig.trim_matches(|c| c == '(' || c == ')');
    let mut parts = sig.split(',');
    
    let x = parts.next().unwrap().trim();
    let y = parts.next().unwrap().trim();
    
    // Convert string numbers to BigUint
    let x_big_int = x.chars()
        .filter(|c| c.is_numeric())
        .collect::<String>()
        .parse::<BigUint>()
        .unwrap();
    let y_big_int = y.chars()
        .filter(|c| c.is_numeric())
        .collect::<String>()
        .parse::<BigUint>()
        .unwrap();
        
    // Create G1Affine point and convert to Signature
     G1Affine::new(x_big_int.into(), y_big_int.into())
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let _a = run_aggregator().await;
}

pub async fn run_aggregator() -> eyre::Result<()> {
    init_logger(eigen_logging::log_level::LogLevel::Info);
    let app = Router::new()
        .route("/aggregate", post(aggregate_sigs));

    info!("Starting aggregator server on port 5074...");
    
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:5074").await?,
        app
    )
    .await
    .map_err(|e| eyre::eyre!("Server error: {}", e))?;

    Ok(())
}

async fn aggregate_sigs(input: String) -> Json<serde_json::Value> {
    info!("Aggregating signatures...");
    
    // First parse to get the outer JSON structure
    let outer_parsed: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to parse outer JSON: {}", e);
            return Json(serde_json::Value::default());
        }
    };

    // Get the inner JSON string and parse it
    let inner_json = match outer_parsed.as_str() {
        Some(s) => s,
        None => {
            error!("Failed to get inner JSON string");
            return Json(serde_json::Value::default());
        }
    };

    // Parse the inner JSON
    let parsed_response: serde_json::Value = match serde_json::from_str(inner_json) {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to parse inner JSON: {}", e);
            return Json(serde_json::Value::default());
        }
    };
    
    debug!("Parsed JSON structure: {:#?}", parsed_response);

    // Now check for the fields
    match parsed_response.get("signature") {
        Some(sig) => debug!("Found signature field: {:?}", sig),
        None => error!("No signature field found in JSON"),
    }

    match parsed_response.get("operator_address") {
        Some(addr) => debug!("Found operator_address field: {:?}", addr),
        None => error!("No operator_address field found in JSON"),
    }

    match parsed_response.get("operator_id") {
        Some(hash) => debug!("Found operator_id field: {:?}", hash),
        None => error!("No operator_id field found in JSON"),
    }

    match parsed_response.get("commitment_hash") {
        Some(hash) => debug!("Found commitment_hash field: {:?}", hash),
        None => error!("No commitment_hash field found in JSON"),
    }

    match parsed_response.get("task_index") {
        Some(task_index) => debug!("Found task_index field: {:?}", task_index),
        None => error!("No task_index field found in JSON"),
    }

    let signature = parsed_response["signature"].as_str().unwrap_or("not found");
    let operator_address = parsed_response["operator_address"].as_str().unwrap_or("not found");
    let operator_id = parsed_response["operator_id"].as_str().unwrap_or("not found");
    let commitment_hash = parsed_response["commitment_hash"].as_str().unwrap_or("not found");
    let task_index = parsed_response["task_index"]
        .as_u64()
        .unwrap_or(0);

    debug!("Extracted values:");
    debug!("Signature: {}", signature);
    debug!("Operator address: {}", operator_address); 
    debug!("Commitment hash: {}", commitment_hash);
    debug!("Task index: {}", task_index);

    // Get addresses and URLs from environment variables
    let registry_coordinator_address: Address = Address::from_str(
        &env::var("REGISTRY_COORDINATOR_ADDRESS")
            .expect("REGISTRY_COORDINATOR_ADDRESS must be set")
    ).unwrap();
    info!("Registry coordinator address: {}", registry_coordinator_address);

    let operator_state_retriever_address: Address = Address::from_str(
        &env::var("OPERATOR_STATE_RETRIEVER_ADDRESS")
            .expect("OPERATOR_STATE_RETRIEVER_ADDRESS must be set")
    ).unwrap();
    info!("Operator state retriever address: {}", operator_state_retriever_address);
    let http_endpoint = env::var("RPC_URL")
        .expect("RPC_URL must be set");
    info!("RPC URL: {}", http_endpoint);
    let ws_endpoint = env::var("WEBSOCKET_RPC_URL")
        .expect("WEBSOCKET_RPC_URL must be set");
    info!("WebSocket URL: {}", ws_endpoint);

    let provider: RootProvider<_, Ethereum> = RootProvider::new_http(Url::parse(&http_endpoint).unwrap());
    // let provider = Provider::new_http(Url::parse(&http_endpoint).unwrap());
    let current_block_num = provider.get_block_number().await.unwrap();
    let quorum_nums = Bytes::from([0x00]);
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![0];
    let time_to_expiry = Duration::from_secs(1000);

    // Create avs clients to interact with contracts deployed on anvil
    let avs_registry_reader = AvsRegistryChainReader::new(
        get_logger().clone(),
        registry_coordinator_address,
        operator_state_retriever_address,
        http_endpoint.clone(),
    ).await.unwrap();
    // let quorums_avs_state = avs_registry_reader.get_quorums_avs_state_at_block(quorum_nums.to_vec(), current_block_num as u32).await.unwrap();
    // info!("Quorums avs state: {:?}", quorums_avs_state);
    let operators_stake = avs_registry_reader.get_operators_stake_in_quorums_at_block(current_block_num as u32, quorum_nums.clone()).await.unwrap();
    info!("Operators stake");
    info!("Number of operators: {}", operators_stake.len());
    info!("First operator stake: {:?}", operators_stake[0][0].stake);
    info!("First operator stake: {:?}", operators_stake[0][1].stake);
    info!("First operator stake: {:?}", operators_stake[0][2].stake);
    let get_check_signatures_indices = avs_registry_reader.get_check_signatures_indices(current_block_num as u32, quorum_nums.to_vec(), vec![]).await.unwrap();
    info!("Get check signatures indices: {:?}", get_check_signatures_indices.nonSignerQuorumBitmapIndices);
    info!("Get check signatures indices: {:?}", get_check_signatures_indices.totalStakeIndices);
    
    let operator_id_mock = avs_registry_reader.get_operator_id(Address::from_str(operator_address).unwrap()).await.unwrap();
    info!("Operator id mock: {:?}", operator_id_mock);
    // let get_quorums_avs_state_at_block = avs_registry_reader.get_quorums_avs_state_at_block(quorum_nums.to_vec(), current_block_num as u32).await.unwrap();
    // info!("Get quorums avs state at block: {:?}", get_quorums_avs_state_at_block);
    // let (operators_info, _rx) = OperatorInfoServiceInMemory::new(
    //     get_logger(),
    //     avs_registry_reader.clone(),
    //     ws_endpoint,
    // ).await.unwrap();
    let operators_info_service = OperatorInfoServiceInMemory::new(
        get_logger(),
        avs_registry_reader.clone(),
        ws_endpoint,
    )
    .await.unwrap().0;
    let operators_info = operators_info_service.get_operator_info(Address::from_str(operator_address).unwrap()).await.unwrap();
    info!("Operators info: {:?}", operators_info);
    let token = tokio_util::sync::CancellationToken::new();
    let current_block_number = provider.get_block_number().await.unwrap();
    let operators_info_service_clone = operators_info_service.clone();
    tokio::spawn(async move {
        let _ = operators_info_service
            .start_service(&token, 0, current_block_number)
            .await;
    });
    let avs_registry_service_chaincaller = AvsRegistryServiceChainCaller::new(
        avs_registry_reader,
        operators_info_service_clone,
    );
    let quorums_avs_state = avs_registry_service_chaincaller.get_quorums_avs_state_at_block(&quorum_nums, current_block_num as u32).await.unwrap();
    info!("Quorums avs state: {:?}", quorums_avs_state);
    let operators_avs_state = avs_registry_service_chaincaller.get_operators_avs_state_at_block(current_block_num as u32, &quorum_nums).await.unwrap();
    info!("Operators avs state: {:?}", operators_avs_state);
    let get_check_signatures_indices = avs_registry_service_chaincaller.get_check_signatures_indices(current_block_num as u32, quorum_nums.to_vec(), vec![]).await.unwrap();
    info!("Get check signatures indices: {:?}", get_check_signatures_indices.nonSignerQuorumBitmapIndices);
    info!("Get check signatures indices: {:?}", get_check_signatures_indices.totalStakeIndices);



    let bls_agg_service = BlsAggregatorService::new(
        avs_registry_service_chaincaller,
        get_logger()
    );


    // Initialize the task
    info!("Initializing task...");
    let task_metadata = TaskMetadata::new(
        task_index as u32,
        current_block_num as u64,
        quorum_nums.to_vec(),
        quorum_threshold_percentages,
        time_to_expiry
    );

    bls_agg_service
        .initialize_new_task(task_metadata)
        .await
        .unwrap();
    
    info!("Processing signature...");
    // Process the signature
    let process_result = bls_agg_service
        .process_new_signature(TaskSignature::new(
            task_index as u32,
            TaskResponseDigest::from(commitment_hash.parse::<FixedBytes<32>>().unwrap()),
            Signature::new(parse_signature(signature)),
            FixedBytes::from_str(operator_id).unwrap(),
        ))
        .await;
    info!("Process result: {:?}", process_result);
    match process_result {
        Ok(_) => {
            info!("Successfully processed signature");
            
            // Wait for aggregated response with timeout
            info!("Waiting for aggregated response...");
            match tokio::time::timeout(
                Duration::from_secs(10),
                bls_agg_service.aggregated_response_receiver.lock().await.recv()
            ).await {
                Ok(Some(Ok(response))) => {
                    debug!("BLS aggregation response: {:?}", response);
                    // Convert to stringified format
                    let stringified = serde_json::json!({
                        "task_index": response.task_index,
                        "task_response_digest": response.task_response_digest,
                        "sig_g1_x": response.signers_agg_sig_g1.g1_point().g1().x().unwrap().into_bigint().to_string(),
                        "sig_g1_y": response.signers_agg_sig_g1.g1_point().g1().y().unwrap().into_bigint().to_string(),
                        "apk_g1_x": response.quorum_apks_g1[0].g1().x().unwrap().into_bigint().to_string(),
                        "apk_g1_y": response.quorum_apks_g1[0].g1().y().unwrap().into_bigint().to_string(),
                        "apk_g2_x2": response.signers_apk_g2.g2().x().unwrap().c0.into_bigint().to_string(),
                        "apk_g2_x1": response.signers_apk_g2.g2().x().unwrap().c1.into_bigint().to_string(),
                        "apk_g2_y2": response.signers_apk_g2.g2().y().unwrap().c0.into_bigint().to_string(),
                        "apk_g2_y1": response.signers_apk_g2.g2().y().unwrap().c1.into_bigint().to_string(),
                        "non_signer_bitmap_indices": response.non_signer_quorum_bitmap_indices,
                        "non_signer_public_keys": response.non_signers_pub_keys_g1
                            .iter()
                            .map(|key| {
                                serde_json::json!({
                                    "x": key.g1().x().unwrap().into_bigint().to_string(),
                                    "y": key.g1().y().unwrap().into_bigint().to_string()
                                })
                            })
                            .collect::<Vec<_>>(),
                        "quorum_apk_indices": response.quorum_apk_indices,
                        "total_stake_indices": response.total_stake_indices,
                        "non_signer_stake_indices": response.non_signer_stake_indices
                    });
                    Json(stringified)
                }
                Ok(Some(Err(e))) => {
                    error!("Error in aggregation response: {:?}", e);
                    Json(serde_json::Value::default())
                }
                Ok(None) => {
                    error!("Aggregation channel closed without response");
                    
                    Json(serde_json::Value::default())
                }
                Err(_) => {
                    error!("Timeout waiting for aggregation response");
                    Json(serde_json::Value::default())
                }
            }
        }
        Err(e) => {
            error!("Failed to process signature: {:?}", e);
            Json(serde_json::Value::default())
        }
    }
}