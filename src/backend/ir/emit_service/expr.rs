//! Expression emission façade, delegating to `IrEmitter`.

use super::EmitService;
use crate::backend::ir::EmitError;
use crate::backend::ir::IrProgram;

impl<'a> EmitService<'a> {
    pub fn emit_program(&mut self, ir: &'a IrProgram) -> Result<String, EmitError> {
        // Delegate to legacy emitter for now.
        self.inner_mut().emit_program(ir)
    }
}
