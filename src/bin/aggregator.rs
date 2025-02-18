use ark_bn254::g1::G1Affine;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::BlsAggregatorService;
use eigen_client_avsregistry::reader::AvsRegistryChainReader;
use eigen_logging::{get_logger, init_logger};
use eigen_logging::logger::Logger;
use eigen_logging::noop_logger::NoopLogger;
use eigen_services_avsregistry::chaincaller::AvsRegistryServiceChainCaller;
use eigen_services_operatorsinfo::operatorsinfo_inmemory::OperatorInfoServiceInMemory;
use eigen_crypto_bls::Signature;
use eigen_types::{
    avs::TaskResponseDigest,
    operator::QuorumThresholdPercentages,
};
use alloy_primitives::{Address, Bytes, FixedBytes, address};
use alloy_provider::{Provider, RootProvider};
use alloy_network::Ethereum;
use url::Url;
use std::time::Duration;
use std::str::FromStr;
use std::sync::Arc;
use ark_ec::AffineRepr;
use ark_ff::PrimeField;
use once_cell::sync::OnceCell;
use eigen_services_blsaggregation::bls_aggregation_service_response::BlsAggregationServiceResponse;
use eigen_services_blsaggregation::bls_aggregation_service_error::BlsAggregationServiceError;

use axum::{
    routing::post,
    Json,
    Router,
};
use tracing::{info, debug, error};
use tokio_util::sync::CancellationToken;
use tokio::{task, time::sleep};
use eigen_services_avsregistry::AvsRegistryService;

static OPERATORS_INFO: OnceCell<Arc<OperatorInfoServiceInMemory>> = OnceCell::new();
static BLS_AGG_SERVICE: OnceCell<Arc<BlsAggregatorService<AvsRegistryServiceChainCaller<AvsRegistryChainReader, OperatorInfoServiceInMemory>>>> = OnceCell::new();
static CANCELLATION_TOKEN: OnceCell<CancellationToken> = OnceCell::new();

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
    let _a = run_aggregator().await;
}

pub async fn run_aggregator() -> eyre::Result<()> {
    init_logger(eigen_logging::log_level::LogLevel::Info);
    
    // Initialize services
    initialize_services().await?;
    
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

async fn initialize_services() -> eyre::Result<()> {
    let registry_coordinator_address: Address = address!("eCd099fA5048c3738a5544347D8cBc8076E76494").into();
    let operator_state_retriever_address: Address = address!("D5D7fB4647cE79740E6e83819EFDf43fa74F8C31").into();
    let http_endpoint = String::from("http://ethereum:8545");
    let ws_endpoint = String::from("ws://ethereum:8545");

    let provider: RootProvider<_, Ethereum> = RootProvider::new_http(Url::parse(&http_endpoint).unwrap());
    let current_block_num = provider.get_block_number().await.unwrap();
    let quorum_nums = Bytes::from([0u8]);
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![33];
    let time_to_expiry = Duration::from_secs(1000);

    // Create avs clients
    let avs_registry_reader = AvsRegistryChainReader::new(
        get_logger().clone(),
        registry_coordinator_address,
        operator_state_retriever_address,
        http_endpoint.clone(),
    ).await.unwrap();

    let (operators_info, _rx) = OperatorInfoServiceInMemory::new(
        get_logger(),
        avs_registry_reader.clone(),
        ws_endpoint,
    ).await.unwrap();

    let cancellation_token = CancellationToken::new();
    let token_clone = cancellation_token.clone();
    let operators_info_clone = operators_info.clone();

    // Store the cancellation token
    CANCELLATION_TOKEN.set(cancellation_token).unwrap();

    // Spawn the operator info service
    task::spawn(async move {
        let _ = operators_info_clone.start_service(&token_clone, current_block_num - 100 as u64, 0).await;
    });

    info!("waiting for services to initialize...");
    sleep(Duration::from_secs(2)).await;

    let avs_registry_service = AvsRegistryServiceChainCaller::new(
        avs_registry_reader.clone(), 
        operators_info.clone()
    );

    let logger: Arc<dyn Logger> = Arc::new(NoopLogger {});
    let bls_agg_service = BlsAggregatorService::new(
        avs_registry_service.clone(),
        logger
    );

    // Store the services in the OnceCell
    OPERATORS_INFO.set(Arc::new(operators_info)).unwrap();
    BLS_AGG_SERVICE.set(Arc::new(bls_agg_service)).unwrap();

    Ok(())
}

async fn aggregate_sigs(input: String) -> Json<serde_json::Value> {
    info!("Aggregating signatures...");
    
    // Get the services from OnceCell
    let bls_agg_service = BLS_AGG_SERVICE.get().unwrap();
    
    // Define the variables needed for task initialization
    let quorum_nums = Bytes::from([0u8]);
    let time_to_expiry = Duration::from_secs(1000);

    // Get current block number
    let provider: RootProvider<_, Ethereum> = RootProvider::new_http(
        Url::parse("http://ethereum:8545").unwrap()
    );
    let current_block_num = provider.get_block_number().await.unwrap();

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

    info!("Processing signature for task_index: {}", task_index);
    info!("Attempting to process signature through BLS aggregation service...");

    debug!("Extracted values:");
    debug!("Signature: {}", signature);
    debug!("Operator address: {}", operator_address); 
    debug!("Commitment hash: {}", commitment_hash);
    debug!("Task index: {}", task_index);

    // Extract threshold from input
    let threshold = parsed_response["threshold"]
        .as_u64()
        .unwrap_or(33) as u8;
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![threshold];

    // Only initialize the task if it fails with TaskNotFound
    match bls_agg_service
        .process_new_signature(
            task_index as u32,
            TaskResponseDigest::from(commitment_hash.parse::<FixedBytes<32>>().unwrap()),
            Signature::new(parse_signature(signature)),
            FixedBytes::from_str(operator_id).unwrap(),
        )
        .await
    {
        Err(BlsAggregationServiceError::TaskNotFound) => {
            info!("Task not found, initializing new task...");
            debug!("Task index: {:?}", task_index);
            debug!("Current block number: {:?}", current_block_num);
            debug!("Quorum nums: {:?}", quorum_nums);
            debug!("Quorum threshold percentages: {:?}", quorum_threshold_percentages);
            debug!("Time to expiry: {:?}", time_to_expiry);
    bls_agg_service
        .initialize_new_task(
                    task_index as u32,
            current_block_num as u32,
            quorum_nums.to_vec(),
            quorum_threshold_percentages,
            time_to_expiry,
        )
        .await
        .unwrap();
            
            // Now try processing the signature again
            bls_agg_service
                .process_new_signature(
                    task_index as u32,
                    TaskResponseDigest::from(commitment_hash.parse::<FixedBytes<32>>().unwrap()),
                    Signature::new(parse_signature(signature)),
                    FixedBytes::from_str(operator_id).unwrap(),
                )
        .await
        .unwrap();
            // Return empty JSON for now, wait for next signature
            Json(serde_json::Value::default())
        }
        Err(BlsAggregationServiceError::ChannelError) => {
            info!("Channel error for task {}, continuing to process signature...", task_index);
            // Try processing the signature again after a short delay
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            // Get a fresh lock on the channel
            debug!("Attempting retry for task {} with operator {}", task_index, operator_id);
            let retry_result = match bls_agg_service
                .process_new_signature(
                    task_index as u32,
                    TaskResponseDigest::from(commitment_hash.parse::<FixedBytes<32>>().unwrap()),
                    Signature::new(parse_signature(signature)),
                    FixedBytes::from_str(operator_id).unwrap(),
                )
        .await
            {
                Ok(_) => {
                    info!("Successfully processed signature after retry for task {}", task_index);
                    // Wait for aggregated response
                    debug!("Waiting for aggregated response after retry...");
                    match tokio::time::timeout(
                        Duration::from_secs(10),
                        bls_agg_service.aggregated_response_receiver.lock().await.recv()
                    ).await {
                        Ok(Some(Ok(response))) => {
                            debug!("BLS aggregation response after retry: {:?}", response);
                            Some(response)
                        }
                        Ok(Some(Err(e))) => {
                            error!("Channel received error after retry: {:?}", e);
                            None
                        }
                        Ok(None) => {
                            error!("Channel closed after retry");
                            None
                        }
                        Err(e) => {
                            error!("Timeout waiting for response after retry: {:?}", e);
                            None
                        }
                    }
                },
                Err(e) => {
                    error!("Failed to process signature for task {} after retry: {:?}", task_index, e);
                    None
                }
            };
            
            match retry_result {
                Some(response) => {
                    info!("Successfully got aggregated response for task {}", task_index);
                    // Convert response to JSON format
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
                None => {
                    error!("No aggregated response available for task {} after retry", task_index);
                    Json(serde_json::Value::default())
                }
            }
        }
        Ok(_) => {
            info!("Successfully processed signature for existing task");
            
            // Wait for aggregated response with timeout
            info!("Waiting for aggregated response...");
            match tokio::time::timeout(
                Duration::from_secs(10),
                bls_agg_service.aggregated_response_receiver.lock().await.recv()
            ).await {
                Ok(Some(Ok(response))) => {
                    debug!("BLS aggregation response: {:?}", response);
                    if let Some(token) = CANCELLATION_TOKEN.get() {
                        token.cancel();
                    }
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
                    if let Some(token) = CANCELLATION_TOKEN.get() {
                        token.cancel();
                    }
                    Json(serde_json::Value::default())
                }
                Ok(None) => {
                    error!("Aggregation channel closed without response");
                    if let Some(token) = CANCELLATION_TOKEN.get() {
                        token.cancel();
                    }
                    Json(serde_json::Value::default())
                }
                Err(_) => {
                    error!("Timeout waiting for aggregation response");
                    if let Some(token) = CANCELLATION_TOKEN.get() {
                        token.cancel();
                    }
                    Json(serde_json::Value::default())
                }
            }
        }
        Err(e) => {
            error!("Error processing signature: {:?}", e);
            return Json(serde_json::Value::default());
        }
    }
}


