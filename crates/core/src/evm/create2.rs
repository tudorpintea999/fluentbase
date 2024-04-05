use crate::{
    account::Account,
    fluent_host::FluentHost,
    helpers::{calc_create2_address, DefaultEvmSpec},
};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Bytes, ExitCode, B256};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, BytecodeLocked,
    Contract, Interpreter, SharedMemory, MAX_CODE_SIZE,
};

#[no_mangle]
pub fn _evm_create2(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    salt32_offset: *const u8,
    address20_offset: *mut u8,
    gas_limit: u32,
) -> ExitCode {
    // TODO: "gas calculations"
    // TODO: "load account so it needs to be marked as warm for access list"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    let is_static = ExecutionContext::contract_is_static();
    if is_static {
        return ExitCode::WriteProtection;
    }

    // read value input
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let salt = B256::from_slice(unsafe { &*ptr::slice_from_raw_parts(salt32_offset, 32) });

    let init_code = unsafe { &*ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let mut init_code_hash = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        init_code.as_ptr(),
        init_code.len() as u32,
        init_code_hash.as_mut_ptr(),
    );

    // load deployer and contract accounts
    let caller_address = ExecutionContext::contract_caller();
    let mut caller_account = Account::new_from_jzkt(&caller_address);
    if caller_account.balance < value {
        return ExitCode::InsufficientBalance;
    }
    caller_account.inc_nonce().expect("nonce inc failed");
    let deployed_contract_address = calc_create2_address(&caller_address, &salt, &init_code_hash);
    let mut callee_account = Account::new_from_jzkt(&deployed_contract_address);

    // transfer value from caller to callee
    match Account::transfer(&mut caller_account, &mut callee_account, value) {
        Ok(_) => {}
        Err(exit_code) => return exit_code,
    }

    // create an account
    match Account::create_account(&mut caller_account, &mut callee_account, value) {
        Ok(_) => {}
        Err(exit_code) => return exit_code,
    }

    let analyzed_bytecode = to_analysed(Bytecode::new_raw(Bytes::from_static(init_code)));
    let deployer_bytecode_locked = BytecodeLocked::try_from(analyzed_bytecode).unwrap();
    let hash = deployer_bytecode_locked.hash_slow();

    let contract = Contract {
        input: Bytes::new(),
        bytecode: deployer_bytecode_locked,
        hash,
        address: deployed_contract_address,
        caller: caller_address,
        value,
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, false);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let result = if let Some(v) = interpreter
        .run(shared_memory, &instruction_table, &mut host)
        .into_result_return()
    {
        v
    } else {
        return ExitCode::EVMCreateError;
    };

    if result.is_error() {
        return ExitCode::EVMCreateError;
    } else if result.is_revert() {
        return ExitCode::EVMCreateRevert;
    }

    if result.output.len() > MAX_CODE_SIZE {
        return ExitCode::ContractSizeLimit;
    }

    callee_account.update_source_bytecode(&result.output);
    callee_account.update_rwasm_bytecode(
        &include_bytes!("../../../contracts/assets/evm_loader_contract.rwasm").into(),
    );

    unsafe { ptr::copy(deployed_contract_address.as_ptr(), address20_offset, 20) }

    ExitCode::Ok
}
