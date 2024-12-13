// call this from run_prover.sh passing "response" as an argument.
// extract the PublicKey and the Signature from the response (this needs the G2 also?)
// aggregate the pk and the signature
// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)
use ark_bn254::g1::G1Affine;
extern crate num;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregationServiceResponse, BlsAggregatorService};
use eigen_client_avsregistry::reader::AvsRegistryChainReader;
use eigen_logging::get_test_logger;
use eigen_services_avsregistry::chaincaller::AvsRegistryServiceChainCaller;
use eigen_services_operatorsinfo::operatorsinfo_inmemory::OperatorInfoServiceInMemory;
use eigen_crypto_bls::{Signature, BlsG1Point};
use eigen_types::{
    avs::TaskIndex,
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
    },
};
use alloy_primitives::{hex, Bytes, FixedBytes};
use alloy_provider::Provider;
use std::time::Duration;

use axum::{
    extract::State,
    routing::post,
    Json,
    Router,
};
use eyre::Result;
use tracing::{info, debug};

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
    let mut commitment_hash: FixedBytes<32> = FixedBytes::default();
    let mut counter = 0;
    for word in &words {
        counter += 1;
        if word.contains("Signature:") {
            signature = parse_to_g1_affine(words[counter], words[counter+1].clone());
            println!("Signature: {:?}", signature);
        }
        if word.contains("OperatorID:") {
            let operator_id_string = words[counter]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        operator_id = FixedBytes::try_from(operator_id_string.as_bytes()).unwrap();
        println!("OperatorID: {:?}", operator_id);
        }
        if word.contains("CommitmentHash:") {
            let commitment_hash_string = words[counter]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        commitment_hash = FixedBytes::try_from(commitment_hash_string.as_bytes()).unwrap();
        println!("CommitmentHash: {:?}", commitment_hash);
        }
    }

    let registry_coordinator_address = hex!("eCd099fA5048c3738a5544347D8cBc8076E76494").into();
    let operator_state_retriever_address = hex!("D5D7fB4647cE79740E6e83819EFDf43fa74F8C31").into();
    let http_endpoint = String::from("http://localhost:8545");
    let ws_endpoint = String::from("ws://localhost:8545");

    // Create avs clients to interact with contracts deployed on anvil
    let avs_registry_reader = AvsRegistryChainReader::new(
        get_test_logger(),
        registry_coordinator_address,
        operator_state_retriever_address,
        http_endpoint.clone(),
    )
    .await
    .unwrap();

    let operators_info = OperatorInfoServiceInMemory::new(
        get_test_logger(),
        avs_registry_reader.clone(),
        ws_endpoint,
    )
    .await
    .unwrap()
    .0;

    // Create aggregation service
    let avs_registry_service =
    AvsRegistryServiceChainCaller::new(avs_registry_reader.clone(), operators_info);

    let bls_agg_service = BlsAggregatorService::new(avs_registry_service);

    let provider = get_provider(http_endpoint.as_str());
    let current_block_num = provider.get_block_number().await.unwrap();
    let task_index = 0;
    let quorum_nums = Bytes::from([0u8]);
    let quorum_threshold_percentages: QuorumThresholdPercentages = vec![33];
    let time_to_expiry = Duration::from_secs(1000);

    // Initialize the task
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

        bls_agg_service
            .process_new_signature(
                task_index,
                commitment_hash,
                Signature::new(signature),
                operator_id,
            )
            .await
            .unwrap();

}