use super::MainWindow;
use crate::graphics::{GraphicsBuilder, PhysicalDevice};
use crate::profile::{DisplayResolution, Profile};
use slint::{Model, ModelNotify, ModelTracker, SharedString, ToSharedString};
use std::any::Any;
use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use thiserror::Error;

/// Implementation of [`Model`] for [`PhysicalDevice`].
pub struct DeviceModel<G>(Rc<G>);

impl<G: GraphicsBuilder> DeviceModel<G> {
    pub fn new(g: Rc<G>) -> Self {
        Self(g)
    }

    pub fn position(&self, id: &[u8]) -> Option<i32> {
        self.0
            .physical_devices()
            .iter()
            .position(move |d| d.id() == id)
            .map(|i| i.try_into().unwrap())
    }

    pub fn get(&self, i: i32) -> Option<&impl PhysicalDevice> {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.0.physical_devices().get(i))
    }
}

impl<G: GraphicsBuilder> Model for DeviceModel<G> {
    type Data = SharedString;

    fn row_count(&self) -> usize {
        self.0.physical_devices().len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.0
            .physical_devices()
            .get(row)
            .map(|d| SharedString::from(d.name()))
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Implementation of [`Model`] for [`DisplayResolution`].
pub struct ResolutionModel([DisplayResolution; 3]);

impl ResolutionModel {
    pub fn position(&self, v: DisplayResolution) -> Option<i32> {
        self.0
            .iter()
            .position(move |i| *i == v)
            .map(|v| v.try_into().unwrap())
    }

    pub fn get(&self, i: i32) -> Option<DisplayResolution> {
        usize::try_from(i).ok().and_then(|i| self.0.get(i)).copied()
    }
}

impl Default for ResolutionModel {
    fn default() -> Self {
        Self([
            DisplayResolution::Hd,
            DisplayResolution::FullHd,
            DisplayResolution::UltraHd,
        ])
    }
}

impl Model for ResolutionModel {
    type Data = SharedString;

    fn row_count(&self) -> usize {
        self.0.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.0.get(row).map(|v| v.to_string().into())
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Implementation of [`Model`] for [`Profile`].
pub struct ProfileModel<G> {
    profiles: RefCell<Vec<Profile>>,
    devices: Rc<DeviceModel<G>>,
    resolutions: Rc<ResolutionModel>,
    noti: ModelNotify,
}

impl<G: GraphicsBuilder> ProfileModel<G> {
    pub fn new(
        profiles: Vec<Profile>,
        devices: Rc<DeviceModel<G>>,
        resolutions: Rc<ResolutionModel>,
    ) -> Self {
        Self {
            profiles: RefCell::new(profiles),
            devices,
            resolutions,
            noti: ModelNotify::default(),
        }
    }

    /// # Panics
    /// If `row` is not valid.
    pub fn select(&self, row: usize, dst: &MainWindow) {
        let profiles = self.profiles.borrow();
        let p = &profiles[row];

        dst.set_selected_device(self.devices.position(p.display_device()).unwrap_or(0));
        dst.set_selected_resolution(self.resolutions.position(p.display_resolution()).unwrap());
        dst.set_debug_address(p.debug_addr().to_shared_string());
    }

    /// # Panics
    /// If `row` is not valid.
    pub fn update(&self, row: i32, src: &MainWindow) -> Result<RefMut<Profile>, ProfileError> {
        let row = usize::try_from(row).unwrap();
        let mut profiles = self.profiles.borrow_mut();
        let p = &mut profiles[row];

        p.set_display_device(self.devices.get(src.get_selected_device()).unwrap().id());
        p.set_display_resolution(self.resolutions.get(src.get_selected_resolution()).unwrap());

        match src.get_debug_address().parse() {
            Ok(v) => p.set_debug_addr(v),
            Err(_) => return Err(ProfileError::InvalidDebugAddress),
        }

        Ok(RefMut::map(profiles, move |v| &mut v[row]))
    }

    pub fn into_inner(self) -> Vec<Profile> {
        self.profiles.into_inner()
    }
}

impl<G: 'static> Model for ProfileModel<G> {
    type Data = SharedString;

    fn row_count(&self) -> usize {
        self.profiles.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.profiles.borrow().get(row).map(|p| p.name().into())
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.noti
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Represents an error when [`ProfileModel::update()`] fails.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid debug address")]
    InvalidDebugAddress,
}
