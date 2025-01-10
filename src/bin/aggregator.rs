// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)
use ark_bn254::g1::G1Affine;
use ark_bn254::Fr;
extern crate num;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregationServiceResponse, BlsAggregatorService};
use eigen_client_avsregistry::reader::{AvsRegistryChainReader, AvsRegistryReader};
use eigen_logging::{get_logger, init_logger};
use eigen_services_avsregistry::{chaincaller::AvsRegistryServiceChainCaller, AvsRegistryService};
use eigen_services_operatorsinfo::{operatorsinfo_inmemory::OperatorInfoServiceInMemory, operator_info::OperatorInfoService};
use eigen_crypto_bls::{Signature, BlsG1Point};
use eigen_types::{
    avs::{TaskIndex, TaskResponseDigest, SignedTaskResponseDigest},
    operator::{QuorumNum, QuorumThresholdPercentages},
};
use eigen_utils::{
    get_provider, get_signer,
    {
        iblssignaturechecker::{
            IBLSSignatureChecker::{self, NonSignerStakesAndSignature},
            BN254::G1Point,
        },
        registrycoordinator::{
            IRegistryCoordinator::OperatorSetParam, IStakeRegistry::StrategyParams,
            RegistryCoordinator,
        },
        operatorstateretriever::OperatorStateRetriever
    },
};
use eigen_crypto_bn254::utils::verify_message;
use alloy_primitives::{hex, Bytes, FixedBytes, address, Address};
use alloy_provider::Provider;
use std::time::Duration;
use std::str::FromStr;

use axum::{
    extract::State,
    routing::post,
    Json,
    Router,
};
use eyre::Result;
use tracing::{info, debug};
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

async fn aggregate_sigs(input: String) {
    println!("Aggregating signatures...");
    
    // First parse to get the outer JSON structure
    let outer_parsed: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            println!("Failed to parse outer JSON: {}", e);
            return;
        }
    };

    // Get the inner JSON string and parse it
    let inner_json = match outer_parsed.as_str() {
        Some(s) => s,
        None => {
            println!("Failed to get inner JSON string");
            return;
        }
    };

    // Parse the inner JSON
    let parsed_response: serde_json::Value = match serde_json::from_str(inner_json) {
        Ok(v) => v,
        Err(e) => {
            println!("Failed to parse inner JSON: {}", e);
            return;
        }
    };
    
    println!("Parsed JSON structure: {:#?}", parsed_response);

    // Now check for the fields
    match parsed_response.get("signature") {
        Some(sig) => println!("Found signature field: {:?}", sig),
        None => println!("No signature field found in JSON"),
    }

    match parsed_response.get("operator_address") {
        Some(addr) => println!("Found operator_address field: {:?}", addr),
        None => println!("No operator_address field found in JSON"),
    }

    match parsed_response.get("operator_id") {
        Some(hash) => println!("Found operator_id field: {:?}", hash),
        None => println!("No operator_id field found in JSON"),
    }

    match parsed_response.get("commitment_hash") {
        Some(hash) => println!("Found commitment_hash field: {:?}", hash),
        None => println!("No commitment_hash field found in JSON"),
    }

    match parsed_response.get("task_index") {
        Some(task_index) => println!("Found task_index field: {:?}", task_index),
        None => println!("No task_index field found in JSON"),
    }

    let signature = parsed_response["signature"].as_str().unwrap_or("not found");
    let operator_address = parsed_response["operator_address"].as_str().unwrap_or("not found");
    let operator_id = parsed_response["operator_id"].as_str().unwrap_or("not found");
    let commitment_hash = parsed_response["commitment_hash"].as_str().unwrap_or("not found");
    let task_index = parsed_response["task_index"]
        .as_u64()
        .unwrap_or(0);

    println!("Extracted values:");
    println!("Signature: {}", signature);
    println!("Operator address: {}", operator_address); 
    println!("Commitment hash: {}", commitment_hash);
    println!("Task index: {}", task_index);

    let registry_coordinator_address: Address = address!("eCd099fA5048c3738a5544347D8cBc8076E76494").into(); // TODO: get from config
    let operator_state_retriever_address: Address = address!("D5D7fB4647cE79740E6e83819EFDf43fa74F8C31").into(); // TODO: get from config
    let http_endpoint = String::from("http://ethereum:8545"); // TODO: get from .env
    let ws_endpoint = String::from("ws://ethereum:8545"); // TODO: get from .env

    let provider = get_provider(http_endpoint.as_str());
    let current_block_num = provider.get_block_number().await.unwrap();
    let quorum_nums = Bytes::from([0u8]);
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![33];
    let time_to_expiry = Duration::from_secs(1000);

    // Create avs clients to interact with contracts deployed on anvil
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

    // Create a channel for coordinating shutdown
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let cancellation_token: CancellationToken = CancellationToken::new();
    let token_clone = cancellation_token.clone();
    let operators_info_clone = operators_info.clone();

    // Spawn the operator info service
    task::spawn(async move { 
        let _ = operators_info_clone.start_service(&token_clone, current_block_num - 100 as u64, 0).await;
        let _ = tx.send(()).await;  // Signal that the service has stopped
    });

    println!("waiting for services to initialize...");
    sleep(Duration::from_secs(2)).await;

    let avs_registry_service = AvsRegistryServiceChainCaller::new(
        avs_registry_reader.clone(), 
        operators_info.clone()
    );

    let bls_agg_service = BlsAggregatorService::new(avs_registry_service.clone());

    // Initialize the task
    println!("Initializing task...");
    println!("Task index: {:?}", task_index);
    println!("Current block number: {:?}", current_block_num);
    println!("Quorum nums: {:?}", quorum_nums);
    println!("Quorum threshold percentages: {:?}", quorum_threshold_percentages);
    println!("Time to expiry: {:?}", time_to_expiry);
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
    println!("Task initialized.");

    println!("Processing signature...");
    println!("Task index: {:?}", task_index);
    println!("Commitment hash: {:?}", commitment_hash);
    println!("Signature: {:?}", signature);
    println!("Operator ID: {:?}", operator_id);
    
    // Process the signature
    let process_result = bls_agg_service
        .process_new_signature(
            task_index as u32,
            TaskResponseDigest::from(commitment_hash.parse::<FixedBytes<32>>().unwrap()),
            Signature::new(parse_signature(signature)),
            FixedBytes::from_str(operator_id).unwrap(),
        )
        .await;

    match process_result {
        Ok(_) => {
            println!("Successfully processed signature");
            
            // Wait for aggregated response with timeout
            println!("Waiting for aggregated response...");
            match tokio::time::timeout(
                Duration::from_secs(10),  // 10 second timeout
                bls_agg_service.aggregated_response_receiver.lock().await.recv()
            ).await {
                Ok(Some(Ok(response))) => {
                    println!("BLS aggregation response: {:?}", response);
                }
                Ok(Some(Err(e))) => {
                    println!("Error in aggregation response: {:?}", e);
                }
                Ok(None) => {
                    println!("Aggregation channel closed without response");
                }
                Err(_) => {
                    println!("Timeout waiting for aggregation response");
                }
            }
        }
        Err(e) => {
            println!("Failed to process signature: {:?}", e);
        }
    }

    // Clean shutdown
    cancellation_token.cancel();
    if rx.recv().await.is_none() {
        println!("Service shutdown channel closed unexpectedly");
    }
}


