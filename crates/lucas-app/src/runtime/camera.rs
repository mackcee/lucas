//! Source lifetime seam; production calls it only on its capture thread.
use lucas_gaze::{GazeSample, GazeSource};

pub struct CaptureSession<S> {
    source: Option<S>,
}
impl<S: GazeSource> Default for CaptureSession<S> {
    fn default() -> Self {
        Self { source: None }
    }
}
impl<S: GazeSource> CaptureSession<S> {
    pub fn set_wanted(
        &mut self,
        wanted: bool,
        open: impl FnOnce() -> lucas_gaze::Result<S>,
    ) -> lucas_gaze::Result<()> {
        if !wanted {
            self.source = None;
        } else if self.source.is_none() {
            self.source = Some(open()?);
        }
        Ok(())
    }
    pub fn next_sample(&mut self) -> lucas_gaze::Result<Option<GazeSample>> {
        let Some(source) = &mut self.source else {
            return Ok(None);
        };
        let result = source.next_sample();
        if !matches!(result, Ok(Some(_))) {
            self.source = None;
        }
        result
    }
}
