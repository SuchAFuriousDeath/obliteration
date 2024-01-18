use super::LlvmEngine;
use crate::{llvm::module::LlvmModule, rtld::Module};
use iced_x86::{Decoder, IcedError};
use thiserror::Error;

/// Contains states for lifting a module.
pub(super) struct Codegen<'a> {
    output: &'a mut LlvmModule,
    module: &'a Module<LlvmEngine>,
}

impl<'a> Codegen<'a> {
    pub fn new(output: &'a mut LlvmModule, module: &'a Module<LlvmEngine>) -> Self {
        Self { output, module }
    }

    pub fn lift(&mut self, offset: usize) -> Result<(), LiftError> {
        use iced_x86::Mnemonic::*;

        let mut decoder = Decoder::try_with_ip(
            64,
            unsafe { self.module.memory().as_bytes() },
            offset as u64,
            0,
        )?;

        self.output.create_entry_function(offset);

        while decoder.can_decode() {
            let i = decoder.decode();

            match i.mnemonic() {
                Add => todo!(),
                Mov => todo!(),
                Push => todo!(),
                _ => todo!("Unimplemented instruction: {:?}", i.code()),
            }
        }

        Ok(())
    }
}

/// Represents an error for [`Codegen::lift()`].
#[derive(Debug, Error)]
pub enum LiftError {
    #[error("failed to create decoder")]
    FailedToCreateDecoder(#[from] IcedError),
}
