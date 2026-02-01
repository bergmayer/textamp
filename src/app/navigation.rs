//! Navigation and screen management.

use super::state::Screen;

/// Navigation controller.
#[derive(Debug, Default)]
pub struct Navigation {
    stack: Vec<Screen>,
}

impl Navigation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a screen onto the stack.
    pub fn push(&mut self, screen: Screen) {
        self.stack.push(screen);
    }

    /// Pop the top screen.
    pub fn pop(&mut self) -> Option<Screen> {
        self.stack.pop()
    }

    /// Peek at the top screen.
    pub fn peek(&self) -> Option<&Screen> {
        self.stack.last()
    }

    /// Clear the navigation stack.
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Check if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Get the depth of the navigation stack.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}
