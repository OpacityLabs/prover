// call this from run_prover.sh passing "response" as an argument.
// extract the PublicKey and the Signature from the response (this needs the G2 also?)
// aggregate the pk and the signature
// once threshold is reached, send the aggregated signature to something onchain (eas maybe?)

fn main() {
    let args: Vec<String> = std::env::args().collect();
    //iterate through args to find the signature and privateKey
    let mut signature = "".to_string();
    let mut public_key = "".to_string();
    let mut counter = 0;
    for arg in &args {
        counter += 1;
        // println!("arg #{}: {}", counter, arg);
        if arg.contains("Signature:") {
            signature = args[counter].clone();
            signature.push_str(&args[counter+1]);
            // println!("Found Signature: {}", signature);
        }
        if arg.contains("PublicKey:") {
            public_key = args[counter].clone();
            public_key.push_str(&args[counter+1]);
            // println!("Found PublicKey: {}", public_key);
        }
    }
}