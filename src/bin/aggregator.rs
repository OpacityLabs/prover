// call this from run_prover.sh passing "response" as an argument.
// extract the PublicKey and the Signature from the response (this needs the G2 also?)
// aggregate the pk and the signature
// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)
use ark_bn254::{G1Affine, G2Affine};
extern crate num;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregationServiceResponse, BlsAggregatorService};

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
    let mut signature: G1Affine;
    let mut operator_id: String;
    let mut counter = 0;
    for word in &words {
        counter += 1;
        if word.contains("Signature:") {
            signature = parse_to_g1_affine(words[counter], words[counter+1].clone());
            println!("Signature: {:?}", signature);
        }
        if word.contains("OperatorID:") {
            operator_id = words[counter]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        println!("OperatorID: {:?}", operator_id);
        }
    }
}