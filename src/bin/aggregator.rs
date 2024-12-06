// call this from run_prover.sh passing "response" as an argument.
// extract the PublicKey and the Signature from the response (this needs the G2 also?)
// aggregate the pk and the signature
// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)
use ark_bn254::{G1Affine, G2Affine};
extern crate num;
use num::bigint::BigUint;
use eigen_services_blsaggregation::bls_agg::{BlsAggregationServiceResponse, BlsAggregatorService};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut signature: G1Affine;
    let mut operator_id: String;
    let mut counter = 0;
    for arg in &args {
        counter += 1;
        if arg.contains("Signature:") {
            signature = parse_to_g1_affine(args[counter].clone(), args[counter+1].clone());
            println!("Signature(yay): {:?}", signature);
        }
        if arg.contains("OperatorID:") {
            operator_id = args[counter]
            .clone()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        println!("OperatorID(yay): {:?}", operator_id);
        }
    }
}

fn parse_to_g1_affine(x: String, y: String) -> G1Affine {
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

