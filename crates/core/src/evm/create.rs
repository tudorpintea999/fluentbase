use crate::helpers::{debug_log, exec_evm_bytecode, exit_code_from_evm_error};
use crate::{fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use alloc::format;
use fluentbase_sdk::{
    Account, AccountManager, ContextReader, EvmCreateMethodInput, EvmCreateMethodOutput,
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, ExitCode, B256};
use revm_interpreter::InstructionResult;
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Bytecode, Bytes},
    return_ok, BytecodeLocked, Contract, Interpreter, SharedMemory, MAX_CODE_SIZE,
};
use revm_primitives::U256;

pub fn _evm_create<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    debug_log("ecl(_evm_create): start");

    // check write protection
    let is_static = cr.contract_is_static();
    if is_static {
        debug_log(&format!(
            "ecl(_evm_create): return: Err: exit_code: {}",
            ExitCode::WriteProtection
        ));
        return EvmCreateMethodOutput::from_exit_code(ExitCode::WriteProtection);
    }

    // load deployer and contract accounts
    let caller_address = cr.contract_caller();
    let (mut caller_account, _) = am.account(caller_address);

    // calc source code hash
    let mut source_code_hash: B256 = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        input.bytecode.as_ptr(),
        input.bytecode.len() as u32,
        source_code_hash.as_mut_ptr(),
    );

    // create journal checkpoint
    let checkpoint = am.checkpoint();

    // create an account
    let salt_hash = input.salt.map(|salt| (salt, source_code_hash));
    let mut callee_account =
        match Account::create_account(am, &mut caller_account, input.value, salt_hash) {
            Ok(result) => result,
            Err(err) => {
                return EvmCreateMethodOutput::from_exit_code(err);
            }
        };

    let analyzed_bytecode = to_analysed(Bytecode::new_raw(input.bytecode.into()));
    let deployer_bytecode_locked = BytecodeLocked::try_from(analyzed_bytecode).unwrap();

    let contract = Contract {
        input: Bytes::new(),
        bytecode: deployer_bytecode_locked,
        hash: source_code_hash,
        address: callee_account.address,
        caller: caller_address,
        value: input.value,
    };

    let result = exec_evm_bytecode(cr, am, contract, input.gas_limit, is_static);

    if !matches!(result.result, return_ok!()) {
        am.rollback(checkpoint);
        debug_log(&format!(
            "ecl(_evm_create): return: Err: {:?}",
            result.result
        ));
        return EvmCreateMethodOutput::from_exit_code(exit_code_from_evm_error(result.result))
            .with_gas(result.gas.remaining());
    }
    if !result.output.is_empty() && result.output.first() == Some(&0xEF) {
        am.rollback(checkpoint);
        debug_log(&format!(
            "ecl(_evm_create): return: Err: {:?}",
            result.result
        ));
        return EvmCreateMethodOutput::from_exit_code(ExitCode::CreateContractStartingWithEF)
            .with_gas(result.gas.remaining());
    }
    if result.output.len() > MAX_CODE_SIZE {
        am.rollback(checkpoint);
        debug_log(&format!(
            "ecl(_evm_create): return: Err: {:?}",
            result.result
        ));
        return EvmCreateMethodOutput::from_exit_code(ExitCode::ContractSizeLimit)
            .with_gas(result.gas.remaining());
    }

    // write caller changes to database
    am.write_account(&caller_account);

    // write callee changes to database
    let evm_loader = Bytes::default();

    callee_account.update_bytecode(am, &result.output, None, &evm_loader, None);

    debug_log(&format!(
        "ecl(_evm_create): return: Ok: callee_account.address: {}",
        callee_account.address
    ));

    am.commit();

    return EvmCreateMethodOutput::from_exit_code(ExitCode::Ok)
        .with_gas(result.gas.remaining())
        .with_address(callee_account.address);
}
