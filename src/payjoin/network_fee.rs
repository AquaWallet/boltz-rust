const FEE_RATE: f64 = 0.1;
const FEE_RATE_LOWBALL: f64 = 0.01;

const WEIGHT_FIXED: usize = 44;
const WEIGHT_VIN_SINGLE_SIG: usize = 367;
const WEIGHT_VIN_MULTI_SIG: usize = 526;
const WEIGHT_VOUT: usize = 4810;
const WEIGHT_FEE: usize = 178;

/// `vout_count` must not count the network fee output
pub fn expected_network_fee(
    single_sig_inputs: usize,
    multi_sig_inputs: usize,
    blinded_outputs: usize,
    is_lowball: bool,
) -> u64 {
    let weight = WEIGHT_FIXED
        + WEIGHT_VIN_SINGLE_SIG * single_sig_inputs
        + WEIGHT_VIN_MULTI_SIG * multi_sig_inputs
        + WEIGHT_VOUT * blinded_outputs
        + WEIGHT_FEE;
    let vsize = (weight + 3) / 4;
    let fee = if is_lowball { FEE_RATE_LOWBALL } else { FEE_RATE };
    (vsize as f64 * fee).ceil() as u64
}
