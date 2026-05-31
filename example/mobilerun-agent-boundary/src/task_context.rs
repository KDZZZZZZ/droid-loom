use crate::context_stability::{mark_stability, ContextStability};
use agent_core::content_block::ContentBlock;
use agent_core::run_message::RunMessage;
use agent_core::{AgentCoreError, AgentCoreResult};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskContextPackage {
    pub task_id: String,
    pub goal: String,
    pub inputs: Vec<String>,
    pub artifacts: Vec<String>,
    pub required_state: Vec<String>,
}

impl TaskContextPackage {
    pub fn new(task_id: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            goal: goal.into(),
            inputs: Vec::new(),
            artifacts: Vec::new(),
            required_state: Vec::new(),
        }
    }

    pub fn with_inputs(mut self, inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.inputs = inputs.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_artifacts(
        mut self,
        artifacts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.artifacts = artifacts.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_required_state(
        mut self,
        required_state: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.required_state = required_state.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskNode {
    pub package: TaskContextPackage,
    pub depends_on: Vec<String>,
}

impl TaskNode {
    pub fn new(package: TaskContextPackage) -> Self {
        Self {
            package,
            depends_on: Vec::new(),
        }
    }

    pub fn depends_on(mut self, task_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.depends_on = task_ids.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct TaskDependencyGraph {
    tasks: BTreeMap<String, TaskNode>,
}

impl TaskDependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_task(&mut self, task: TaskNode) -> AgentCoreResult<()> {
        let task_id = task.package.task_id.clone();
        if task_id.trim().is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "task id must not be empty".to_string(),
            ));
        }
        if self.tasks.insert(task_id.clone(), task).is_some() {
            return Err(AgentCoreError::InvalidInput(format!(
                "duplicate task id: {task_id}"
            )));
        }
        Ok(())
    }

    pub fn context_for(&self, task_id: &str) -> AgentCoreResult<Vec<RunMessage>> {
        let task = self.task(task_id)?;
        let dependencies = self.dependency_order(task_id)?;
        let mut messages = Vec::new();

        for dependency_id in dependencies {
            let dependency = self.task(&dependency_id)?;
            if dependency.package.artifacts.is_empty() {
                continue;
            }
            messages.push(mark_stability(
                package_message(
                    "Dependency result",
                    &dependency.package.task_id,
                    &dependency.package.goal,
                    &[],
                    &dependency.package.artifacts,
                    &[],
                )?,
                ContextStability::DependencyResult,
            ));
        }

        messages.push(mark_stability(
            package_message(
                "Task package",
                &task.package.task_id,
                &task.package.goal,
                &task.package.inputs,
                &[],
                &task.package.required_state,
            )?,
            ContextStability::TaskPackage,
        ));

        Ok(messages)
    }

    pub fn dependency_edges(&self) -> Vec<(String, String)> {
        self.tasks
            .values()
            .flat_map(|task| {
                task.depends_on
                    .iter()
                    .map(|dependency| (dependency.clone(), task.package.task_id.clone()))
            })
            .collect()
    }

    fn task(&self, task_id: &str) -> AgentCoreResult<&TaskNode> {
        self.tasks
            .get(task_id)
            .ok_or_else(|| AgentCoreError::NotFound(format!("task not found: {task_id}")))
    }

    fn dependency_order(&self, task_id: &str) -> AgentCoreResult<Vec<String>> {
        let mut ordered = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        self.visit_dependencies(task_id, &mut visiting, &mut visited, &mut ordered)?;
        Ok(ordered)
    }

    fn visit_dependencies(
        &self,
        task_id: &str,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> AgentCoreResult<()> {
        if visited.contains(task_id) {
            return Ok(());
        }
        if !visiting.insert(task_id.to_string()) {
            return Err(AgentCoreError::InvalidConfig(format!(
                "task dependency cycle includes: {task_id}"
            )));
        }

        let task = self.task(task_id)?;
        for dependency_id in &task.depends_on {
            self.visit_dependencies(dependency_id, visiting, visited, ordered)?;
            if !ordered.contains(dependency_id) {
                ordered.push(dependency_id.clone());
            }
        }

        visiting.remove(task_id);
        visited.insert(task_id.to_string());
        Ok(())
    }
}

pub fn sample_mobile_task_dag() -> AgentCoreResult<TaskDependencyGraph> {
    let mut graph = TaskDependencyGraph::new();
    graph.add_task(TaskNode::new(
        TaskContextPackage::new("open_app", "Open Demo Shop and wait for the search UI.")
            .with_artifacts(["app_open=true", "screen=home"]),
    ))?;
    graph.add_task(
        TaskNode::new(
            TaskContextPackage::new("search_catalog", "Search for the requested product.")
                .with_inputs(["search_term=wireless charger"])
                .with_artifacts(["first_result=Wireless Charger Stand"])
                .with_required_state(["screen=home"]),
        )
        .depends_on(["open_app"]),
    )?;
    graph.add_task(
        TaskNode::new(
            TaskContextPackage::new("finalize", "Return the remembered product summary.")
                .with_required_state(["first_result is known"]),
        )
        .depends_on(["search_catalog"]),
    )?;
    Ok(graph)
}

fn package_message(
    label: &str,
    task_id: &str,
    goal: &str,
    inputs: &[String],
    artifacts: &[String],
    required_state: &[String],
) -> AgentCoreResult<RunMessage> {
    let mut lines = vec![format!("{label}: {task_id}"), format!("Goal: {goal}")];
    if !inputs.is_empty() {
        lines.push(format!("Inputs: {}", inputs.join("; ")));
    }
    if !artifacts.is_empty() {
        lines.push(format!("Artifacts: {}", artifacts.join("; ")));
    }
    if !required_state.is_empty() {
        lines.push(format!("Required state: {}", required_state.join("; ")));
    }

    Ok(
        RunMessage::user(vec![ContentBlock::text(lines.join("\n"))])?
            .with_metadata("task.id", serde_json::json!(task_id)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_task_inherits_only_dependency_artifacts() {
        let graph = sample_mobile_task_dag().unwrap();
        let messages = graph.context_for("finalize").unwrap();
        let text = serde_json::to_string(&messages).unwrap();

        assert!(text.contains("first_result=Wireless Charger Stand"));
        assert!(text.contains("Task package: finalize"));
        assert!(!text.contains("click_at"));
        assert!(!text.contains("temporary screen"));
    }
}
