use std::collections::{HashMap, VecDeque};

/// A node in the tool execution DAG.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub id: String,
    pub command: String,
    pub is_read_only: bool,
    /// IDs of nodes that must complete before this one can run.
    pub dependencies: Vec<String>,
}

/// A Directed Acyclic Graph for scheduling tool calls.
/// Independent read-only operations are grouped into parallel batches.
/// Mutating operations maintain strict ordering.
#[derive(Debug, Default)]
pub struct ExecutionDag {
    nodes: HashMap<String, DagNode>,
}

impl ExecutionDag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: DagNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Computes a topological execution schedule.
    /// Returns a Vec of "batches" — each batch contains node IDs that can run in parallel.
    pub fn schedule(&self) -> Vec<Vec<String>> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

        // Initialize in-degrees
        for (id, node) in &self.nodes {
            in_degree.entry(id.clone()).or_insert(0);
            for dep in &node.dependencies {
                dependents.entry(dep.clone()).or_default().push(id.clone());
                *in_degree.entry(id.clone()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (id, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(id.clone());
            }
        }

        let mut batches: Vec<Vec<String>> = Vec::new();

        while !queue.is_empty() {
            // All nodes in the current queue have no unmet dependencies
            let mut next_queue: VecDeque<String> = VecDeque::new();

            // Separate read-only (parallelizable) from mutating (must be sequential)
            let mut read_only_batch: Vec<String> = Vec::new();
            let mut mutating_batch: Vec<String> = Vec::new();

            while let Some(id) = queue.pop_front() {
                if let Some(node) = self.nodes.get(&id) {
                    if node.is_read_only {
                        read_only_batch.push(id);
                    } else {
                        mutating_batch.push(id);
                    }
                }
            }

            // All read-only operations go in one parallel batch
            if !read_only_batch.is_empty() {
                // Update dependents
                for id in &read_only_batch {
                    if let Some(deps) = dependents.get(id) {
                        for dep_id in deps {
                            if let Some(degree) = in_degree.get_mut(dep_id) {
                                *degree -= 1;
                                if *degree == 0 {
                                    next_queue.push_back(dep_id.clone());
                                }
                            }
                        }
                    }
                }
                batches.push(read_only_batch);
            }

            // Mutating operations each get their own batch (strict ordering)
            for id in mutating_batch {
                if let Some(deps) = dependents.get(&id) {
                    for dep_id in deps {
                        if let Some(degree) = in_degree.get_mut(dep_id) {
                            *degree -= 1;
                            if *degree == 0 {
                                next_queue.push_back(dep_id.clone());
                            }
                        }
                    }
                }
                batches.push(vec![id]);
            }

            queue = next_queue;
        }

        batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_dag_parallel_and_sequential_batches() {
        let mut dag = ExecutionDag::new();

        // Two independent read-only tasks
        dag.add_node(DagNode {
            id: "read_1".into(),
            command: "cat file1".into(),
            is_read_only: true,
            dependencies: vec![],
        });
        dag.add_node(DagNode {
            id: "read_2".into(),
            command: "cat file2".into(),
            is_read_only: true,
            dependencies: vec![],
        });

        // A mutating task dependent on both reads
        dag.add_node(DagNode {
            id: "write_1".into(),
            command: "echo test > file3".into(),
            is_read_only: false,
            dependencies: vec!["read_1".into(), "read_2".into()],
        });

        let schedule = dag.schedule();
        assert_eq!(schedule.len(), 2);
        // Batch 1 has both read tasks in parallel
        assert_eq!(schedule[0].len(), 2);
        assert!(schedule[0].contains(&"read_1".to_string()));
        assert!(schedule[0].contains(&"read_2".to_string()));
        // Batch 2 has the mutating write task
        assert_eq!(schedule[1], vec!["write_1".to_string()]);
    }
}
