use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
use retoken_core::config::SandboxMode;
use crate::task::Task;

pub struct TaskManager {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    sandbox_mode: SandboxMode,
}

impl TaskManager {
    pub fn new(sandbox_mode: SandboxMode) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            sandbox_mode,
        }
    }

    /// Submits a task and spawns it in the background. Returns the task ID.
    pub async fn submit(&self, command: String) -> String {
        let id = Uuid::new_v4().to_string();
        let mut task = Task::new(id.clone(), command, self.sandbox_mode.clone());
        
        // Register the task as pending
        {
            let mut map = self.tasks.lock().await;
            map.insert(id.clone(), task.clone());
        }

        // Spawn background execution
        let tasks_ref = self.tasks.clone();
        let task_id = id.clone();
        
        tokio::spawn(async move {
            // Wait for it to execute
            let _ = task.execute().await;
            
            // Update the map with the final state
            let mut map = tasks_ref.lock().await;
            map.insert(task_id, task);
        });

        id
    }

    /// Retrieve the current state of a task
    pub async fn get_task(&self, id: &str) -> Option<Task> {
        let map = self.tasks.lock().await;
        map.get(id).cloned()
    }
}
