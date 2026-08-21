use super::App;
use crate::frontend::chat::Role;
use crate::frontend::tools::ToolStatus;
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(90);

pub(super) struct Demo {
    step: u8,
    due: Instant,
    read: Option<usize>,
    bash: Option<usize>,
}

impl Demo {
    pub(super) fn new() -> Self {
        Self {
            step: 0,
            due: Instant::now(),
            read: None,
            bash: None,
        }
    }
}

impl App {
    pub fn tick(&mut self) {
        let (step, read, bash) = {
            let Some(demo) = self.demo.as_ref() else {
                return;
            };
            if Instant::now() < demo.due {
                return;
            }
            (demo.step, demo.read, demo.bash)
        };
        match step {
            0 => {
                self.chat.push(
                    Role::Assistant,
                    "I'll inspect the entrypoint and run a quick check.".into(),
                );
                let read = self.chat.start_tool("read", "src/raid/main.rs");
                self.advance_demo(1, Some(read), None);
            }
            1 => {
                if let Some(read) = read {
                    self.chat.finish_tool(
                        read,
                        ToolStatus::Success,
                        "Read 42 lines (ctrl+r to expand)",
                    );
                }
                let bash = self.chat.start_tool("bash", "cargo test --offline");
                self.advance_demo(2, None, Some(bash));
            }
            2 => {
                if let Some(bash) = bash {
                    self.chat.finish_tool(
                        bash,
                        ToolStatus::Success,
                        "Tests passed (ctrl+r to expand)",
                    );
                }
                self.chat.push(Role::Assistant, REPLY.into());
                self.demo = None;
            }
            _ => self.demo = None,
        }
    }

    fn advance_demo(&mut self, step: u8, read: Option<usize>, bash: Option<usize>) {
        if let Some(demo) = self.demo.as_mut() {
            demo.step = step;
            if let Some(read) = read {
                demo.read = Some(read);
            }
            if let Some(bash) = bash {
                demo.bash = Some(bash);
            }
            demo.due = Instant::now() + STEP;
        }
    }

    #[cfg(test)]
    pub(super) fn run_demo_to_end(&mut self) {
        let mut guard = 0;
        while self.demo.is_some() && guard < 8 {
            if let Some(demo) = self.demo.as_mut() {
                demo.due = Instant::now();
            }
            self.tick();
            guard += 1;
        }
    }
}

const REPLY: &str = "\
**Done.** `main.rs` still boots the shell, and the offline tests came back clean.

- chat sits at the top
- tools sit in the timeline
- thinking slot is reserved

```rs
fn run(terminal: &mut DefaultTerminal)
```

Want the real agent loop next?";
