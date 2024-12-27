// call this from run_prover.sh passing "response" as an argument.
// extract the PublicKey and the Signature from the response (this needs the G2 also?)
// aggregate the pk and the signature
// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)
use ark_bn254::g1::G1Affine;
extern crate num;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregationServiceResponse, BlsAggregatorService};
use eigen_client_avsregistry::reader::{AvsRegistryChainReader, AvsRegistryReader};
use eigen_logging::{get_logger, init_logger};
use eigen_services_avsregistry::{chaincaller::AvsRegistryServiceChainCaller, AvsRegistryService};
use eigen_services_operatorsinfo::{operatorsinfo_inmemory::OperatorInfoServiceInMemory, operator_info::OperatorInfoService};
use eigen_crypto_bls::{Signature, BlsG1Point};
use eigen_types::{
    avs::{TaskIndex, TaskResponseDigest},
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

pub fn parse_to_g1_affine(x: &str, y: &str) -> G1Affine {
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
    G1Affine::new(x_big_int.into(), y_big_int.into())
}

#[tokio::main]
async fn main() {
    let a = run_aggregator().await;
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
    // println!("Received signature: {:?}", input);
    let words: Vec<&str> = input.split_whitespace().collect();
    let mut signature: G1Affine = G1Affine::default();
    let mut operator_id: FixedBytes<32> = FixedBytes::default();
    let mut commitment_hash: TaskResponseDigest = FixedBytes::default();
    let mut counter = 0;
    for word in &words {
        counter += 1;
        if word.contains("Signature:") {
            signature = parse_to_g1_affine(words[counter], words[counter+1]);
            println!("Signature: {:?}", signature);
        }
        if word.contains("OperatorID:") {
            let operator_id_string = words[counter]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        operator_id = operator_id_string.parse::<FixedBytes<32>>().unwrap();
        println!("OperatorID: {:?}", operator_id);
        }
        if word.contains("CommitmentHash:") {
            let commitment_hash_string = words[counter]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        commitment_hash = commitment_hash_string.parse::<FixedBytes<32>>().unwrap();
        println!("CommitmentHash: {:?}", commitment_hash);
        }
    }

    let registry_coordinator_address: Address = address!("eCd099fA5048c3738a5544347D8cBc8076E76494").into(); // TODO: get from config
    let operator_state_retriever_address: Address = address!("D5D7fB4647cE79740E6e83819EFDf43fa74F8C31").into(); // TODO: get from config
    let http_endpoint = String::from("http://ethereum:8545"); // TODO: get from .env
    let ws_endpoint = String::from("ws://ethereum:8545"); // TODO: get from .env

    // Create avs clients to interact with contracts deployed on anvil
    let avs_registry_reader = AvsRegistryChainReader::new(
        get_logger().clone(),
        registry_coordinator_address,
        operator_state_retriever_address,
        http_endpoint.clone(),
    )
    .await
    .unwrap();

    println!("registry_coordinator_address: {:?}", registry_coordinator_address);
    println!("operator_state_retriever_address: {:?}", operator_state_retriever_address);

    let operators_info = OperatorInfoServiceInMemory::new(
        get_logger(),
        avs_registry_reader.clone(),
        ws_endpoint,
    )
    .await
    .unwrap()
    .0;
    println!("operators_info: {:?}", operators_info);

    let cancellation_token: CancellationToken = CancellationToken::new();
    let token_clone = cancellation_token.clone();
    let operators_info_clone = operators_info.clone();
    task::spawn(async move { operators_info_clone.start_service(&token_clone, 21495071, 0).await }); // TODO: what should the block number be?

    println!("wainting 2 seconds...");
    sleep(Duration::from_secs(2)).await;
    // send cancel token to stop the service
    cancellation_token.cancel();

    // Create aggregation service
    let avs_registry_service =
    AvsRegistryServiceChainCaller::new(avs_registry_reader.clone(), operators_info.clone());

    let bls_agg_service = BlsAggregatorService::new(avs_registry_service.clone());

    let provider = get_provider(http_endpoint.as_str());
    let current_block_num = provider.get_block_number().await.unwrap();
    let task_index = 1;
    let quorum_nums = Bytes::from([0u8]);
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![33];
    let time_to_expiry = Duration::from_secs(1000);

    // Initialize the task
    println!("Initializing task...");
    println!("Task index: {:?}", task_index);
    println!("Current block number: {:?}", current_block_num);
    println!("Quorum nums: {:?}", quorum_nums);
    println!("Quorum threshold percentages: {:?}", quorum_threshold_percentages);
    println!("Time to expiry: {:?}", time_to_expiry);
    bls_agg_service
        .initialize_new_task(
            task_index,
            current_block_num as u32,
            quorum_nums.to_vec(),
            quorum_threshold_percentages,
            time_to_expiry,
        )
        .await
        .unwrap();
    println!("Task initialized.");

    // let my_operator_id = FixedBytes::from_str("0x239442a339edec7728e9d122fe7aa5c9a31529476a9c86d3c698f308e2ad9007").unwrap();
    // let my_operator_addr = avs_registry_reader.get_operator_from_id(*my_operator_id).await.unwrap();
    // println!("My operator address: {:?}", my_operator_addr);
    let operator_addr = avs_registry_reader.get_operator_from_id(*operator_id).await.unwrap();
    println!("Operator address: {:?}", operator_addr);

    let operator_info = operators_info
        .get_operator_info(operator_addr)
        .await;
    println!("Operator info: {:?}", operator_info.unwrap());

    // let my_operator_info = operators_info
    //     .get_operator_info(my_operator_addr)
    //     .await;
    // println!("My operator info: {:?}", my_operator_info.unwrap());


    println!("getting operator state...");
    // get operator state
    let operator_state = avs_registry_service
        .get_operators_avs_state_at_block(current_block_num as u32, &[0u8])
        .await
        .unwrap();
    println!("Operator state: {:?}", operator_state);
    // println!("DO YOU SEE THIS?");

    let signature_for_agg = Signature::new(signature);

    println!("verifying signature...");
    let signature_result = BlsAggregatorService::verify_signature(
            task_index,
            signature_for_agg,
            operator_state,
        )
        .await
        .unwrap();


    println!("Processing signature...");
    println!("Task index: {:?}", task_index);
    println!("Commitment hash: {:?}", commitment_hash);
    println!("Signature: {:?}", signature);
    println!("Operator ID: {:?}", operator_id);
    bls_agg_service
        .process_new_signature(
            task_index,
            commitment_hash,
            signature_for_agg,
            operator_id,
        )
        .await
        .unwrap();

}