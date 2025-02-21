use crate::{
    debug_log,
    helpers::InputHelper,
    wasm::{call::_wasm_call, create::_wasm_create},
};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    ExecutionContext,
    JzktAccountManager,
    LowLevelSDK,
    SharedAPI,
    WasmCallMethodInput,
    WasmCreateMethodInput,
    WASM_CALL_METHOD_ID,
    WASM_CREATE_METHOD_ID,
};

pub fn deploy() {}

pub fn main() {
    let cr = ExecutionContext::default();
    let am = JzktAccountManager::default();
    let input_helper = InputHelper::new();
    let method_id = input_helper.decode_method_id();
    match method_id {
        WASM_CREATE_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<WasmCreateMethodInput>();
            let method_output = _wasm_create(&cr, &am, method_input);
            LowLevelSDK::write(&method_output.encode_to_vec(0));
        }
        WASM_CALL_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<WasmCallMethodInput>();
            let method_output = _wasm_call(&cr, &am, method_input);
            LowLevelSDK::write(&method_output.encode_to_vec(0));
            debug_log!(
                "wcl: WASM_CALL_METHOD_ID: sys_halt: exit_code: {}",
                method_output.exit_code
            );
        }
        _ => panic!("unknown method id: {}", method_id),
    }
}

#[cfg(test)]
mod tests {
    use fluentbase_codec::Encoder;
    use fluentbase_sdk::{
        ContractInput,
        CoreInput,
        LowLevelSDK,
        WasmCreateMethodInput,
        WASM_CREATE_METHOD_ID,
    };
    use revm_primitives::U256;

    #[test]
    fn test_greeting_deploy() {
        let wasm_bytecode = include_bytes!("../../../../examples/greeting/lib.wasm");
        let core_input = CoreInput {
            method_id: WASM_CREATE_METHOD_ID,
            method_data: WasmCreateMethodInput {
                value: U256::ZERO,
                bytecode: wasm_bytecode.into(),
                gas_limit: 3_000_000,
                salt: None,
                depth: 0,
            },
        };
        let contract_input = core_input.encode_to_vec(0);
        LowLevelSDK::with_test_context(ContractInput::default().encode_to_vec(0));
        LowLevelSDK::with_test_input(contract_input);
        super::main();
        assert!(LowLevelSDK::get_test_output().len() > 0);
    }
}
