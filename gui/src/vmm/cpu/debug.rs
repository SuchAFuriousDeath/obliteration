// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::vmm::arch::GdbRegs;
use std::sync::mpsc::{Receiver, Sender};

pub fn channel() -> (Debuggee, Debugger) {
    let (sender, rx) = std::sync::mpsc::channel();
    let (tx, receiver) = std::sync::mpsc::channel();
    let debuggee = Debuggee::new(sender, receiver);
    let debugger = Debugger {
        receiver: rx,
        sender: tx,
    };

    (debuggee, debugger)
}

/// Encapsulates channels to communicate with a debuggee thread.
///
/// All method need a mutable reference to prevent request-response out of sync.
pub struct Debuggee {
    sender: Sender<DebugReq>,
    receiver: Receiver<DebugRes>,
    locked: bool,
}

impl Debuggee {
    fn new(sender: Sender<DebugReq>, receiver: Receiver<DebugRes>) -> Self {
        Self {
            sender,
            receiver,
            locked: false,
        }
    }

    pub fn get_regs(&mut self) -> Option<GdbRegs> {
        self.sender.send(DebugReq::GetRegs).ok()?;
        self.locked = true;
        self.receiver.recv().ok().map(|v| match v {
            DebugRes::Regs(v) => v,
            _ => panic!("unexpected response when getting registers {v:?}"),
        })
    }

    pub fn translate_address(&mut self, addr: usize) -> Option<usize> {
        self.sender.send(DebugReq::TranslateAddress(addr)).ok()?;

        self.locked = true;

        self.receiver.recv().ok().map(|v| match v {
            DebugRes::TranslatedAddress(v) => v,
            _ => panic!("unexpected response when translating address {v:?}"),
        })
    }

    pub fn lock(&mut self) {
        self.sender.send(DebugReq::Lock).ok();
        self.locked = true;
    }

    pub fn release(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.sender.send(DebugReq::Release).ok();
        }
    }
}

/// Encapsulates channels to communicate with a debugger thread.
pub struct Debugger {
    receiver: Receiver<DebugReq>,
    sender: Sender<DebugRes>,
}

impl Debugger {
    pub fn recv(&self) -> Option<DebugReq> {
        self.receiver.recv().ok()
    }

    pub fn send(&self, r: DebugRes) {
        let _ = self.sender.send(r);
    }
}

/// Debug request from a debugger to a debuggee.
#[derive(Debug)]
pub enum DebugReq {
    GetRegs,
    Lock,
    Release,
    TranslateAddress(usize),
}

/// Debug response from a debuggee to a debugger.
#[derive(Debug)]
pub enum DebugRes {
    Regs(GdbRegs),
    TranslatedAddress(usize),
}
