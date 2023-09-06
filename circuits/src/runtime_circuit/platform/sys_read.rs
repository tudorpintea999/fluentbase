use crate::{
    runtime_circuit::{constraint_builder::OpConstraintBuilder, platform::PlatformGadget},
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use fluentbase_runtime::IMPORT_SYS_READ;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct SysReadGadget<F: Field> {
    pd: PhantomData<F>,
}

impl<F: Field> PlatformGadget<F, { IMPORT_SYS_READ }> for SysReadGadget<F> {
    fn configure(_cb: &mut OpConstraintBuilder<F>) -> Self {
        todo!()
    }

    fn assign_exec_step(
        &self,
        _region: &mut Region<'_, F>,
        _offset: usize,
        _trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        todo!()
    }
}
