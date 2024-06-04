use std::ffi::CStr;
use std::os::raw::c_char;
extern crate elements;
extern crate rand;

use anyhow::{bail, ensure};
use base64::Engine;
use elements::hashes::sha256d;
use std::str::FromStr;
use std::time::Duration;

use elements::{Address, Txid};
use elements::hex::FromHex;
use elements::AssetId;
use elements::{pset::PartiallySignedTransaction, TxOutSecrets};

use super::server_api;
use super::pset;
use super::network_fee::expected_network_fee;

#[repr(C)]
pub struct UtxoFFI {
    pub txid: *const c_char,
    pub vout: u32,
    pub script_pub_key: *const c_char,
    pub asset_id: *const c_char,
    pub value: u64,
    pub asset_bf: *const c_char,
    pub value_bf: *const c_char,
    pub asset_commitment: *const c_char,
    pub value_commitment: *const c_char,
}
pub struct CreatedPayjoin {
    pub pset: PartiallySignedTransaction,
    pub asset_fee: u64,
}

pub struct CreatePayjoin {
    pub base_url: String,
    pub user_agent: String,
    pub asset_id: elements::AssetId,
    pub amount: u64,
    pub address: elements::Address,
    pub subtract_fee_from_amount: bool,
    pub change_address: elements::Address,
    pub utxos: Vec<server_api::Utxo>,
}
struct ConstructPsetArgs {
    selected_utxos: Vec<server_api::Utxo>,
    send_address: elements::Address,
    send_asset_id: elements::AssetId,
    send_amount: u64,
    fee_address: elements::Address,
    fixed_fee: u64,
    price: f64,
    client_change_address: elements::Address,
    utxo_asset_amount: u64,
    utxo_lbtc_amount: u64,
    server_change_address: elements::Address,
    lbtc_asset_id: elements::AssetId,
    is_lowball: bool,
}

struct ConstructedPset {
    blinded_pset: PartiallySignedTransaction,
}

pub fn create_taxi_transaction_internal(
    send_amount: u64,
    send_address: *const c_char,
    change_address: *const c_char,
    utxos: *const UtxoFFI,
    utxos_len: usize,
    user_agent: *const c_char,
    api_key: *const c_char,
    subtract_fee_from_amount: bool,
    is_lowball: bool,
    is_testnet: bool,
) -> Result<String, anyhow::Error> {

   let base_url = if is_testnet {
        "https://api-testnet.sideswap.io/"
    } else {
        "https://api.sideswap.io"
    };

    let usdt_asset = if is_testnet {
        AssetId::from_str("b612eb46313a2cd6ebabd8b7a8eed5696e29898b87a43bff41c94f51acef9d73").unwrap()
    } else {
        AssetId::from_str("ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2").unwrap()
    };

    let lbtc_asset_id = if is_testnet {
        AssetId::from_str("144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49").unwrap()
    } else {
        AssetId::from_str("6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d").unwrap()
    };

    let send_asset_id = usdt_asset;

    let client_send_address_str = unsafe { CStr::from_ptr(send_address).to_str().unwrap().trim() };
    let change_address_str = unsafe { CStr::from_ptr(change_address).to_str().unwrap().trim() };
    let client_send_address = Address::from_str(client_send_address_str).expect("Valid address");
    let client_change_address = Address::from_str(change_address_str).expect("Valid address");
    let utxos_slice = unsafe { std::slice::from_raw_parts(utxos, utxos_len) };
    let client_utxos = utxos_slice
        .iter()
        .map(convert_to_native_utxo)
        .collect::<Result<Vec<_>, _>>()?;

    let agent: ureq::Agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();

    let url = format!("{base_url}/payjoin");

    let user_agent_str = unsafe {
        CStr::from_ptr(user_agent).to_str().unwrap_or_default().trim().to_owned()
    };
    let api_key_str = unsafe {
        CStr::from_ptr(api_key).to_str().unwrap_or_default().trim().to_owned()
    };
    let req = server_api::Request::Start(server_api::StartRequest {
        asset_id: usdt_asset,
        user_agent: user_agent_str,
        api_key: api_key_str,
    });
    let resp = make_server_request(&agent, &url, req)?;

    let server_api::StartResponse {
        order_id,
        expires_at: _,
        fee_address,
        change_address: server_change_address,
        utxos: server_utxos,
        price,
        fixed_fee,
    } = match resp {
        server_api::Response::Start(resp) => resp,
        _ => bail!("unexpected response {resp:?}"),
    };

    ensure!(client_send_address.params == fee_address.params);
    ensure!(client_send_address.params == server_change_address.params);
    ensure!(fee_address.is_blinded());
    ensure!(server_change_address.is_blinded());
    ensure!(!server_utxos.is_empty());

    let max_input_count = client_utxos.len() + server_utxos.len();
    let max_output_count = 4;
    let max_network_fee = expected_network_fee(max_input_count, 0, max_output_count, is_lowball);
    log::debug!("max_network_fee: {max_network_fee}");
    let max_asset_fee = fixed_fee + (price * max_network_fee as f64).round() as u64;
    log::debug!("max_asset_fee: {max_asset_fee}");
    let send_amount = if subtract_fee_from_amount {
        ensure!(send_amount > max_asset_fee);
        send_amount - max_asset_fee
    } else {
        send_amount
    };
    log::debug!("send_amount: {send_amount}");

    let mut selected_utxos = Vec::new();

    let mut utxo_asset_amount = 0;
    for utxo in client_utxos.into_iter() {
        log::debug!("client_utxo value {}", utxo.value);
        utxo_asset_amount += utxo.value;
        selected_utxos.push(utxo);

        if utxo_asset_amount >= send_amount + max_asset_fee {
            break;
        }
    }
    let mut utxo_lbtc_amount = 0;
    for utxo in server_utxos.into_iter() {
        utxo_lbtc_amount += utxo.value;
        selected_utxos.push(utxo);
        if utxo_lbtc_amount >= max_network_fee {
            break;
        }
    }
    log::debug!("utxo_asset_amount value {}", utxo_asset_amount);

    ensure!(utxo_asset_amount >= send_amount + max_asset_fee);
    ensure!(utxo_lbtc_amount >= max_network_fee);

    let ConstructedPset { blinded_pset } = construct_pset(ConstructPsetArgs {
        selected_utxos,
        send_address: client_send_address,
        send_asset_id,
        send_amount,
        fee_address,
        fixed_fee,
        price,
        client_change_address,
        utxo_asset_amount,
        utxo_lbtc_amount,
        server_change_address,
        lbtc_asset_id,
        is_lowball,
    })?;

    let server_pset = pset::remove_explicit_values(blinded_pset.clone());
    let server_pset = elements::encode::serialize(&server_pset);

    let req = server_api::Request::Sign(server_api::SignRequest {
        order_id,
        pset: base64::engine::general_purpose::STANDARD.encode(server_pset),
    });
    let resp = make_server_request(&agent, &url, req)?;
    let server_api::SignResponse {
        pset: server_signed_pset,
    } = match resp {
        server_api::Response::Sign(resp) => resp,
        _ => bail!("unexpected response {resp:?}"),
    };
    let server_signed_pset = elements::encode::deserialize::<PartiallySignedTransaction>(
        &base64::engine::general_purpose::STANDARD.decode(server_signed_pset)?,
    )?;

    let pset: PartiallySignedTransaction = pset::copy_signatures(blinded_pset, server_signed_pset)?;
    let serialized_pset = elements::encode::serialize(&pset);
    let base64_encoded_pset = base64::engine::general_purpose::STANDARD.encode(&serialized_pset);
    Ok(base64_encoded_pset)
}

fn make_server_request(
    agent: &ureq::Agent,
    url: &str,
    req: server_api::Request,
) -> Result<server_api::Response, anyhow::Error> {
    let res = agent.post(url).send_json(req);

    match res {
        Ok(resp) => {
            let resp = resp.into_json::<server_api::Response>()?;
            log::debug!("{:?}", resp);
            Ok(resp)
        }
        Err(ureq::Error::Transport(err)) => {
            bail!("unexpected HTTP transport error: {err}");
        }
        Err(ureq::Error::Status(400, resp)) => {
            let err = resp.into_json::<server_api::Error>()?.error;
            bail!("unexpected server error: {err}");
        }
        Err(ureq::Error::Status(status, resp)) => {
            let err = resp.into_string()?;
            bail!("unexpected HTTP status: {status}: {err}");
        }
    }
}

fn construct_pset(args: ConstructPsetArgs) -> Result<ConstructedPset, anyhow::Error> {
    let ConstructPsetArgs {
        selected_utxos,
        send_address,
        send_asset_id,
        send_amount,
        fee_address,
        fixed_fee,
        price,
        client_change_address,
        utxo_asset_amount,
        utxo_lbtc_amount,
        server_change_address,
        lbtc_asset_id,
        is_lowball,
    } = args;

    let mut pset = PartiallySignedTransaction::new_v2();
    let mut input_secrets = Vec::new();

    for utxo in selected_utxos.into_iter() {
        let input = pset::pset_input(pset::PsetInput {
            txid: utxo.txid,
            vout: utxo.vout,
            script_pub_key: utxo.script_pub_key.clone(),
            asset_commitment: utxo.asset_commitment.into(),
            value_commitment: utxo.value_commitment.into(),
        });

        pset.add_input(input);

        input_secrets.push(TxOutSecrets {
            asset: utxo.asset_id,
            asset_bf: utxo.asset_bf,
            value: utxo.value,
            value_bf: utxo.value_bf,
        });
    }

    pset.add_output(pset::pset_output(pset::PsetOutput {
        address: send_address,
        asset: send_asset_id,
        amount: send_amount,
    })?);

    // FIXME: Separate between single-sig and multi-sig inputs
    let network_fee = expected_network_fee(pset.inputs().len(), 0, 4, is_lowball);

    let asset_fee = fixed_fee + (price * network_fee as f64).round() as u64;

    pset.add_output(pset::pset_output(pset::PsetOutput {
        address: fee_address,
        asset: send_asset_id,
        amount: asset_fee,
    })?);

    let asset_change_amount = utxo_asset_amount - send_amount - asset_fee;
    if asset_change_amount > 0 {
        pset.add_output(pset::pset_output(pset::PsetOutput {
            address: client_change_address,
            asset: send_asset_id,
            amount: asset_change_amount,
        })?);
    }

    let lbtc_change_amount = utxo_lbtc_amount - network_fee;
    if lbtc_change_amount > 0 {
        pset.add_output(pset::pset_output(pset::PsetOutput {
            address: server_change_address,
            asset: lbtc_asset_id,
            amount: lbtc_change_amount,
        })?);
    }

    pset.add_output(pset::pset_network_fee(lbtc_asset_id, network_fee));

    let pset = pset::randomize_and_blind_pset(pset, &input_secrets)?;

    Ok(ConstructedPset { blinded_pset: pset })
}

fn convert_to_native_utxo(utxo_ffi: &UtxoFFI) -> Result<server_api::Utxo, anyhow::Error> {
    let txid_cstr = unsafe { CStr::from_ptr(utxo_ffi.txid) };
    let txid_str = txid_cstr.to_str().expect("Invalid UTF-8 string");
    let txid_hash = sha256d::Hash::from_str(txid_str).expect("Invalid hash string");

    let script_pub_key = elements::script::Script::from(unsafe {
        CStr::from_ptr(utxo_ffi.script_pub_key)
            .to_str()?
            .as_bytes()
            .to_vec()
    });

    let asset_id = {
        let asset_id_cstr = unsafe { CStr::from_ptr(utxo_ffi.asset_id) };
        let asset_id_str = asset_id_cstr
            .to_str()
            .map_err(|e| anyhow::anyhow!("Failed to convert asset_id to str: {}", e))?;
        let mut asset_id_bytes = hex::decode(asset_id_str)
            .map_err(|e| anyhow::anyhow!("Failed to decode asset_id: {}", e))?;
        asset_id_bytes.reverse();
        let asset_id = elements::AssetId::from_slice(&asset_id_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create AssetId from bytes: {}", e))?;
        asset_id
    };

    let asset_bf = {
        let asset_bf_str = unsafe { CStr::from_ptr(utxo_ffi.asset_bf).to_str()? };
        elements::confidential::AssetBlindingFactor::from_hex(asset_bf_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse asset_bf: {}", e))?
    };

    let value_bf = {
        let value_bf_str = unsafe { CStr::from_ptr(utxo_ffi.value_bf).to_str()? };
        elements::confidential::ValueBlindingFactor::from_hex(value_bf_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse value_bf: {}", e))?
    };

    let asset_commitment = elements::secp256k1_zkp::Generator::from_str(unsafe {
        CStr::from_ptr(utxo_ffi.asset_commitment).to_str()?
    })?;
    let value_commitment = elements::secp256k1_zkp::PedersenCommitment::from_str(unsafe {
        CStr::from_ptr(utxo_ffi.value_commitment).to_str()?
    })?;

    Ok(server_api::Utxo {
        txid: Txid::from(txid_hash),
        vout: utxo_ffi.vout,
        script_pub_key: script_pub_key,
        asset_id: asset_id,
        value: utxo_ffi.value,
        asset_bf: asset_bf,
        value_bf: value_bf,
        asset_commitment: asset_commitment,
        value_commitment: value_commitment,
    })
}

pub fn final_tx(
    pset_client: PartiallySignedTransaction,
    pset_server: PartiallySignedTransaction,
) -> Result<elements::Transaction, anyhow::Error> {
    let pset = pset::copy_signatures(pset_client, pset_server)?;
    let tx = pset.extract_tx()?;
    Ok(tx)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_create_taxi_transaction_internal() {
        let send_amount = 1000000000;
        let send_address = CString::new(
            "VJLAymK4u1GV39Hf1j8QJST3X6JX2S8Gy4AjJVVbRAMSwQJt6LvgQqvahSphBeaan2LQu3AfcNMKiZEU",
        )
        .expect("CString::new failed");
        let change_address = CString::new(
            "VJLBhFw1eekX14wuDwXjz49Fm9uxCF7XLy7WyQWcaN28pMPBfP8hbLRHyXm85FWNUd6i55C8pejddH2V",
        )
        .expect("CString::new failed");
        let user_agent = CString::new(
            "test",
        )
        .expect("CString::new failed");
        let api_key = CString::new(
            "12345",
        )
        .expect("CString::new failed");

        let utxos = vec![UtxoFFI {
            txid: CString::new("5fab3f795e6a4dbc55413121cfd0ca6d8387afba3ea7d11e5055a40159d2dbfa")
                .unwrap()
                .into_raw(),
            vout: 2,
            script_pub_key: CString::new("76a914e8b4e39dbe9fcf47bf67776754741d3d01bbce9e88ac")
                .unwrap()
                .into_raw(),
            asset_id: CString::new(
                "ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2",
            )
            .unwrap()
            .into_raw(),
            value: 9621587040,
            asset_bf: CString::new(
                "d95dd04d208ee42168ad7b83d1565c250e1a9ca9c36b0bc01a90f3d5c33d5663",
            )
            .unwrap()
            .into_raw(),
            value_bf: CString::new(
                "d0ffde0a3d1201e969c4e6cbdc66746f84e9a1535941e7826d1b850af4b3d925",
            )
            .unwrap()
            .into_raw(),
            asset_commitment: CString::new(
                "0b13337d4f8134158c781defc3605400d076f5f03bbe93c6c5a7e6f6fe9bbc4a5a",
            )
            .unwrap()
            .into_raw(),
            value_commitment: CString::new(
                "0899bc85abb10751f4845b3c473b4b0b069ef7a104364eaaaa9fbb344e7b1cc27e",
            )
            .unwrap()
            .into_raw(),
        }];

        let utxos_len = utxos.len();
        let utxos_ptr = utxos.as_ptr();

        let result = create_taxi_transaction_internal(
            send_amount,
            send_address.as_ptr(),
            change_address.as_ptr(),
            utxos_ptr,
            utxos_len,
            user_agent.as_ptr(),
            api_key.as_ptr(),
            // mock values above are for mainnet
            false,
            false,
        );

        assert!(result.is_ok());      
        log::debug!("Transaction result: {:?}", result);
    }
}
