use cctk::workspace::{WorkspaceHandler, WorkspaceState};
use cosmic::cctk;

use super::{AppData, Event};

impl WorkspaceHandler for AppData {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        // For toplevel screenshots, we don't need complex workspace handling
        // Just send empty workspaces event
        self.send_event(Event::Workspaces(Vec::new()));
    }
}

cctk::delegate_workspace!(AppData);
