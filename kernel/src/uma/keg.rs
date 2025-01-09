/// Implementation of `uma_keg` structure.
pub struct UmaKeg {}

impl UmaKeg {
    /// See `keg_ctor` on the Orbis for a reference.
    ///
    /// # Context safety
    /// This function does not require a CPU context on **stage 1** heap.
    ///
    /// # Reference offsets
    /// | Version | Offset |
    /// |---------|--------|
    /// |PS4 11.00|0x13CF40|
    pub(super) fn new(_: usize) -> Self {
        todo!()
    }
}
