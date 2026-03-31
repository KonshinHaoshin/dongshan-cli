#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub session: String,
    pub last_status: String,
    pub last_output: String,
}

impl RuntimeState {
    pub fn new(session: String, mode: impl Into<String>) -> Self {
        Self {
            session,
            last_status: {
                let _ = mode.into();
                "idle".to_string()
            },
            last_output: String::new(),
        }
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.last_status = status.into();
    }

    pub fn set_output(&mut self, output: impl Into<String>) {
        self.last_output = output.into();
    }
}
