use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::content_block::ContentBlock;
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::run_message::{MessageRole, MessageStatus, RunMessage};

pub type NodeId = String;
pub type EdgeId = String;
pub type OutputPort = String;
pub type InputPackageName = String;
pub type PackageItemName = String;
pub type ContentHash = u64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutputRef {
    pub node: NodeId,
    pub port: OutputPort,
}

impl OutputRef {
    pub fn new(node: impl Into<NodeId>, port: impl Into<OutputPort>) -> Self {
        Self {
            node: node.into(),
            port: port.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageRef {
    pub node: NodeId,
    pub package: InputPackageName,
}

impl PackageRef {
    pub fn new(node: impl Into<NodeId>, package: impl Into<InputPackageName>) -> Self {
        Self {
            node: node.into(),
            package: package.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputLogEntry {
    pub seq: u64,
    pub node: NodeId,
    pub port: OutputPort,
    pub message: RunMessage,
    pub message_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeliveryKey {
    pub edge_id: EdgeId,
    pub package: InputPackageName,
    pub item: PackageItemName,
    pub message_id: Uuid,
    pub selected_hash: ContentHash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeState {
    pub edge_id: EdgeId,
    pub next_seq: u64,
    pub delivered: BTreeSet<DeliveryKey>,
}

impl EdgeState {
    pub fn new(edge_id: impl Into<EdgeId>) -> Self {
        Self {
            edge_id: edge_id.into(),
            next_seq: 0,
            delivered: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: EdgeId,
    pub from: OutputRef,
    pub to: PackageRef,
}

impl GraphEdge {
    pub fn new(
        id: impl Into<EdgeId>,
        from: (impl Into<NodeId>, impl Into<OutputPort>),
        to: (impl Into<NodeId>, impl Into<InputPackageName>),
    ) -> Self {
        Self {
            id: id.into(),
            from: OutputRef::new(from.0, from.1),
            to: PackageRef::new(to.0, to.1),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn from(&self) -> &OutputRef {
        &self.from
    }

    pub fn to(&self) -> &PackageRef {
        &self.to
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPath(pub String);

impl From<&str> for FieldPath {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for FieldPath {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum FieldOp {
    Exists,
    Eq(Value),
    In(Vec<Value>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldFilter {
    pub path: FieldPath,
    pub op: FieldOp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMask {
    pub include: Vec<FieldPath>,
}

impl FieldMask {
    pub fn all() -> Self {
        Self {
            include: Vec::new(),
        }
    }

    pub fn include<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<FieldPath>,
    {
        Self {
            include: paths.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    Latest,
    One,
    AtLeast(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageQuery {
    pub filters: Vec<FieldFilter>,
    pub select: FieldMask,
}

impl MessageQuery {
    pub fn any() -> Self {
        Self {
            filters: Vec::new(),
            select: FieldMask::all(),
        }
    }

    pub fn where_exists(path: impl Into<FieldPath>) -> Self {
        Self::any().filter(path, FieldOp::Exists)
    }

    pub fn where_eq(path: impl Into<FieldPath>, value: impl Into<Value>) -> Self {
        Self::any().filter(path, FieldOp::Eq(value.into()))
    }

    pub fn filter(mut self, path: impl Into<FieldPath>, op: FieldOp) -> Self {
        self.filters.push(FieldFilter {
            path: path.into(),
            op,
        });
        self
    }

    pub fn select<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<FieldPath>,
    {
        self.select = FieldMask::include(paths);
        self
    }

    pub fn matches(&self, message: &RunMessage) -> AgentCoreResult<Option<SelectedFields>> {
        for filter in &self.filters {
            let values = values_at_path(message, &filter.path.0)?;
            if !filter_matches(&values, &filter.op) {
                return Ok(None);
            }
        }

        Ok(Some(select_fields(message, &self.select)?))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectedFields {
    pub value: Value,
    pub hash: ContentHash,
}

fn select_fields(message: &RunMessage, mask: &FieldMask) -> AgentCoreResult<SelectedFields> {
    let value = if mask.include.is_empty() {
        serde_json::to_value(message)?
    } else {
        let mut object = Map::new();
        for path in &mask.include {
            object.insert(
                path.0.clone(),
                values_to_value(values_at_path(message, &path.0)?),
            );
        }
        Value::Object(object)
    };
    let hash = stable_hash(&value)?;
    Ok(SelectedFields { value, hash })
}

fn stable_hash(value: &Value) -> AgentCoreResult<ContentHash> {
    let serialized = serde_json::to_string(value)?;
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    Ok(hasher.finish())
}

fn values_to_value(values: Vec<Value>) -> Value {
    match values.as_slice() {
        [] => Value::Null,
        [single] => single.clone(),
        _ => Value::Array(values),
    }
}

fn filter_matches(values: &[Value], op: &FieldOp) -> bool {
    match op {
        FieldOp::Exists => !values.is_empty() && values.iter().any(|value| !value.is_null()),
        FieldOp::Eq(expected) => values.iter().any(|value| value == expected),
        FieldOp::In(expected) => values
            .iter()
            .any(|value| expected.iter().any(|candidate| candidate == value)),
    }
}

fn values_at_path(message: &RunMessage, path: &str) -> AgentCoreResult<Vec<Value>> {
    if path == "id" {
        return Ok(vec![Value::String(message.id.to_string())]);
    }
    if path == "role" {
        return Ok(vec![Value::String(role_name(message.role).to_string())]);
    }
    if path == "status" {
        return Ok(vec![Value::String(status_name(message.status).to_string())]);
    }
    if path == "source_node_id" {
        return Ok(message
            .source_node_id
            .as_ref()
            .map(|value| vec![Value::String(value.clone())])
            .unwrap_or_default());
    }
    if let Some(key) = path.strip_prefix("metadata.") {
        return Ok(message
            .metadata
            .get(key)
            .cloned()
            .map(|value| vec![value])
            .unwrap_or_default());
    }
    if path == "content[*]" {
        return message
            .content
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into);
    }
    if let Some(field) = path.strip_prefix("content[*].") {
        let mut values = Vec::new();
        for block in &message.content {
            values.extend(content_block_values(block, field)?);
        }
        return Ok(values);
    }

    Err(AgentCoreError::InvalidInput(format!(
        "unsupported message field path: {path}"
    )))
}

fn content_block_values(block: &ContentBlock, field: &str) -> AgentCoreResult<Vec<Value>> {
    match (block, field) {
        (ContentBlock::Text { .. }, "type") => Ok(vec![Value::String("text".to_string())]),
        (ContentBlock::Reasoning { .. }, "type") => {
            Ok(vec![Value::String("reasoning".to_string())])
        }
        (ContentBlock::ToolCall { .. }, "type") => Ok(vec![Value::String("tool_call".to_string())]),
        (ContentBlock::ToolResult { .. }, "type") => {
            Ok(vec![Value::String("tool_result".to_string())])
        }
        (ContentBlock::FileReference { .. }, "type") => {
            Ok(vec![Value::String("file_reference".to_string())])
        }
        (ContentBlock::ImageReference { .. }, "type") => {
            Ok(vec![Value::String("image_reference".to_string())])
        }
        (ContentBlock::AudioReference { .. }, "type") => {
            Ok(vec![Value::String("audio_reference".to_string())])
        }
        (ContentBlock::Diagnostic { .. }, "type") => {
            Ok(vec![Value::String("diagnostic".to_string())])
        }
        (ContentBlock::Custom { .. }, "type") => Ok(vec![Value::String("custom".to_string())]),
        (ContentBlock::Text { text } | ContentBlock::Reasoning { text }, "text") => {
            Ok(vec![Value::String(text.clone())])
        }
        (ContentBlock::ToolCall { call_id, .. }, "call_id")
        | (ContentBlock::ToolResult { call_id, .. }, "call_id") => {
            Ok(vec![Value::String(call_id.clone())])
        }
        (ContentBlock::ToolCall { tool_name, .. }, "tool_name") => {
            Ok(vec![Value::String(tool_name.clone())])
        }
        (ContentBlock::ToolResult { tool_name, .. }, "tool_name") => Ok(tool_name
            .as_ref()
            .map(|value| vec![Value::String(value.clone())])
            .unwrap_or_default()),
        (ContentBlock::ToolCall { arguments, .. }, "arguments") => Ok(vec![arguments.clone()]),
        (ContentBlock::ToolResult { output, .. }, "output") => Ok(vec![output.clone()]),
        (ContentBlock::ToolResult { is_error, .. }, "is_error") => Ok(vec![Value::Bool(*is_error)]),
        (ContentBlock::Custom { value }, "value") => Ok(vec![value.clone()]),
        _ => Ok(Vec::new()),
    }
}

fn role_name(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Diagnostic => "diagnostic",
    }
}

fn status_name(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Streaming => "streaming",
        MessageStatus::Finalized => "finalized",
        MessageStatus::Aborted => "aborted",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageItemSpec {
    pub name: PackageItemName,
    pub query: MessageQuery,
    pub cardinality: Cardinality,
}

impl PackageItemSpec {
    pub fn new(
        name: impl Into<PackageItemName>,
        query: MessageQuery,
        cardinality: Cardinality,
    ) -> Self {
        Self {
            name: name.into(),
            query,
            cardinality,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputPackageSpec {
    pub name: InputPackageName,
    pub required: Vec<PackageItemSpec>,
    pub optional: Vec<PackageItemSpec>,
}

impl InputPackageSpec {
    pub fn new(name: impl Into<InputPackageName>) -> Self {
        Self {
            name: name.into(),
            required: Vec::new(),
            optional: Vec::new(),
        }
    }

    pub fn required(
        mut self,
        name: impl Into<PackageItemName>,
        query: MessageQuery,
        cardinality: Cardinality,
    ) -> Self {
        self.required
            .push(PackageItemSpec::new(name, query, cardinality));
        self
    }

    pub fn optional(
        mut self,
        name: impl Into<PackageItemName>,
        query: MessageQuery,
        cardinality: Cardinality,
    ) -> Self {
        self.optional
            .push(PackageItemSpec::new(name, query, cardinality));
        self
    }

    pub fn item(&self, name: &str) -> Option<(&PackageItemSpec, PackageItemKind)> {
        self.required
            .iter()
            .find(|item| item.name == name)
            .map(|item| (item, PackageItemKind::Required))
            .or_else(|| {
                self.optional
                    .iter()
                    .find(|item| item.name == name)
                    .map(|item| (item, PackageItemKind::Optional))
            })
    }

    fn items(&self) -> impl Iterator<Item = (&PackageItemSpec, PackageItemKind)> {
        self.required
            .iter()
            .map(|item| (item, PackageItemKind::Required))
            .chain(
                self.optional
                    .iter()
                    .map(|item| (item, PackageItemKind::Optional)),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageItemKind {
    Required,
    Optional,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchedContent {
    pub source: OutputRef,
    pub message_id: Uuid,
    pub message_version: u64,
    pub selected: SelectedFields,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageItemState {
    pub matches: Vec<MatchedContent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageState {
    pub node: NodeId,
    pub package: InputPackageName,
    pub version: u64,
    pub items: BTreeMap<PackageItemName, PackageItemState>,
    pub last_activated_version: Option<u64>,
}

impl PackageState {
    pub fn new(node: impl Into<NodeId>, package: impl Into<InputPackageName>) -> Self {
        Self {
            node: node.into(),
            package: package.into(),
            version: 0,
            items: BTreeMap::new(),
            last_activated_version: None,
        }
    }

    fn insert(&mut self, spec: &PackageItemSpec, content: MatchedContent) {
        let state = self.items.entry(spec.name.clone()).or_default();
        match spec.cardinality {
            Cardinality::Latest => {
                state.matches.clear();
                state.matches.push(content);
            }
            Cardinality::One | Cardinality::AtLeast(_) => {
                state.matches.push(content);
            }
        }
        self.version += 1;
    }

    pub fn is_ready(&self, spec: &InputPackageSpec) -> bool {
        spec.required.iter().all(|item| {
            let count = self
                .items
                .get(&item.name)
                .map(|state| state.matches.len())
                .unwrap_or_default();
            match item.cardinality {
                Cardinality::Latest | Cardinality::One => count >= 1,
                Cardinality::AtLeast(required) => count >= required,
            }
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeInput {
    pub package: InputPackageName,
    pub version: u64,
    pub required: BTreeMap<PackageItemName, Vec<MatchedContent>>,
    pub optional: BTreeMap<PackageItemName, Vec<MatchedContent>>,
}

impl NodeInput {
    fn from_package(spec: &InputPackageSpec, state: &PackageState) -> Self {
        let mut required = BTreeMap::new();
        for item in &spec.required {
            required.insert(
                item.name.clone(),
                state
                    .items
                    .get(&item.name)
                    .map(|item_state| item_state.matches.clone())
                    .unwrap_or_default(),
            );
        }

        let mut optional = BTreeMap::new();
        for item in &spec.optional {
            optional.insert(
                item.name.clone(),
                state
                    .items
                    .get(&item.name)
                    .map(|item_state| item_state.matches.clone())
                    .unwrap_or_default(),
            );
        }

        Self {
            package: state.package.clone(),
            version: state.version,
            required,
            optional,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeOutput {
    pub messages: BTreeMap<OutputPort, Vec<RunMessage>>,
}

impl NodeOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_message(mut self, port: impl Into<OutputPort>, message: RunMessage) -> Self {
        self.messages.entry(port.into()).or_default().push(message);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolNodeSpec {
    pub tool_name: String,
    pub call_item: PackageItemName,
    pub result_port: OutputPort,
}

impl ToolNodeSpec {
    pub fn new(
        tool_name: impl Into<String>,
        call_item: impl Into<PackageItemName>,
        result_port: impl Into<OutputPort>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            call_item: call_item.into(),
            result_port: result_port.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentNodeSpec {
    pub agent_name: String,
    pub graph: GraphRef,
}

impl AgentNodeSpec {
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            graph: GraphRef::SelfGraph,
        }
    }

    pub fn with_graph(mut self, graph: GraphRef) -> Self {
        self.graph = graph;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GraphRef {
    SelfGraph,
    Named(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum NodeKind {
    Transform { executor: String, config: Value },
    Tool(ToolNodeSpec),
    Agent(AgentNodeSpec),
    Graph { graph_name: String },
    Final,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConcurrencyKey {
    pub item: PackageItemName,
}

impl ConcurrencyKey {
    pub fn package_item(item: impl Into<PackageItemName>) -> Self {
        Self { item: item.into() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeConcurrency {
    Serial,
    Parallel {
        max: usize,
    },
    ByKey {
        key: ConcurrencyKey,
        max_per_key: usize,
    },
}

impl Default for NodeConcurrency {
    fn default() -> Self {
        Self::Serial
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: NodeId,
    pub kind: NodeKind,
    pub input: InputPackageSpec,
    pub outputs: Vec<OutputPort>,
    pub concurrency: NodeConcurrency,
}

impl NodeSpec {
    pub fn new(id: impl Into<NodeId>, kind: NodeKind, input: InputPackageSpec) -> Self {
        Self {
            id: id.into(),
            kind,
            input,
            outputs: Vec::new(),
            concurrency: NodeConcurrency::Serial,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent(
        id: impl Into<NodeId>,
        agent_name: impl Into<String>,
        input: InputPackageSpec,
    ) -> Self {
        Self::new(id, NodeKind::Agent(AgentNodeSpec::new(agent_name)), input)
    }

    pub fn tool(
        id: impl Into<NodeId>,
        tool_name: impl Into<String>,
        call_item: impl Into<PackageItemName>,
        result_port: impl Into<OutputPort>,
        input: InputPackageSpec,
    ) -> Self {
        Self::new(
            id,
            NodeKind::Tool(ToolNodeSpec::new(tool_name, call_item, result_port)),
            input,
        )
    }

    pub fn final_node(id: impl Into<NodeId>, input: InputPackageSpec) -> Self {
        Self::new(id, NodeKind::Final, input)
    }

    pub fn output(mut self, port: impl Into<OutputPort>) -> Self {
        self.outputs.push(port.into());
        self
    }

    pub fn concurrency(mut self, concurrency: NodeConcurrency) -> Self {
        self.concurrency = concurrency;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphSpec {
    pub name: String,
    pub nodes: BTreeMap<NodeId, NodeSpec>,
    pub edges: Vec<GraphEdge>,
    pub input: OutputRef,
    pub finish_node: Option<NodeId>,
}

impl GraphSpec {
    pub fn builder(name: impl Into<String>) -> GraphSpecBuilder {
        GraphSpecBuilder::new(name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn node(&self, node_id: &str) -> Option<&NodeSpec> {
        self.nodes.get(node_id)
    }

    pub fn nodes(&self) -> &BTreeMap<NodeId, NodeSpec> {
        &self.nodes
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    pub fn input(&self) -> &OutputRef {
        &self.input
    }

    pub fn finish_node(&self) -> Option<&str> {
        self.finish_node.as_deref()
    }
}

#[derive(Clone, Debug)]
pub struct GraphSpecBuilder {
    name: String,
    nodes: BTreeMap<NodeId, NodeSpec>,
    edges: Vec<GraphEdge>,
    input: OutputRef,
    finish_node: Option<NodeId>,
}

impl GraphSpecBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            input: OutputRef::new("input", "messages"),
            finish_node: None,
        }
    }

    pub fn input(mut self, node: impl Into<NodeId>, port: impl Into<OutputPort>) -> Self {
        self.input = OutputRef::new(node, port);
        self
    }

    pub fn node(mut self, node: NodeSpec) -> Self {
        self.nodes.insert(node.id.clone(), node);
        self
    }

    pub fn edge(
        mut self,
        id: impl Into<EdgeId>,
        from: (impl Into<NodeId>, impl Into<OutputPort>),
        to: (impl Into<NodeId>, impl Into<InputPackageName>),
    ) -> Self {
        self.edges.push(GraphEdge::new(id, from, to));
        self
    }

    pub fn finish_at(mut self, node_id: impl Into<NodeId>) -> Self {
        self.finish_node = Some(node_id.into());
        self
    }

    pub fn build(self) -> AgentCoreResult<GraphSpec> {
        if self.name.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "graph runtime spec name must not be empty".to_string(),
            ));
        }

        for edge in &self.edges {
            if edge.from.node != self.input.node && !self.nodes.contains_key(&edge.from.node) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "edge {} source node not found: {}",
                    edge.id, edge.from.node
                )));
            }
            let Some(target) = self.nodes.get(&edge.to.node) else {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "edge {} target node not found: {}",
                    edge.id, edge.to.node
                )));
            };
            if target.input.name != edge.to.package {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "edge {} target package not found: {}.{}",
                    edge.id, edge.to.node, edge.to.package
                )));
            }
        }

        if let Some(finish_node) = &self.finish_node {
            if !self.nodes.contains_key(finish_node) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "finish node not found: {finish_node}"
                )));
            }
        }

        Ok(GraphSpec {
            name: self.name,
            nodes: self.nodes,
            edges: self.edges,
            input: self.input,
            finish_node: self.finish_node,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeActivation {
    pub id: Uuid,
    pub node: NodeId,
    pub package: InputPackageName,
    pub package_version: u64,
    pub input: NodeInput,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphRuntimeState {
    pub output_logs: BTreeMap<OutputRef, Vec<OutputLogEntry>>,
    pub edge_states: BTreeMap<EdgeId, EdgeState>,
    pub package_states: BTreeMap<PackageRef, PackageState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeTransferRecord {
    pub edge_id: EdgeId,
    pub from: OutputRef,
    pub to: PackageRef,
    pub item: PackageItemName,
    pub message_id: Uuid,
    pub message_version: u64,
    pub selected_hash: ContentHash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeAttemptRecord {
    pub attempt_id: Uuid,
    pub node: NodeId,
    pub package_version: u64,
    pub status: NodeAttemptStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAttemptStatus {
    Completed,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphRunLedger {
    pub transfers: Vec<EdgeTransferRecord>,
    pub node_attempts: Vec<NodeAttemptRecord>,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum RuntimeEvent {
    EdgeScanned {
        edge_id: EdgeId,
        from_seq: u64,
    },
    MessageFiltered {
        edge_id: EdgeId,
        seq: u64,
    },
    PackageUpdated {
        node: NodeId,
        package: InputPackageName,
        version: u64,
    },
    NodeActivated {
        node: NodeId,
        package_version: u64,
    },
    NodeCompleted {
        node: NodeId,
    },
    NodeFailed {
        node: NodeId,
        error: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRunInput {
    pub run_id: Option<Uuid>,
    pub initial_messages: Vec<RunMessage>,
    pub max_ticks: usize,
}

impl GraphRunInput {
    pub fn new(initial_messages: Vec<RunMessage>) -> Self {
        Self {
            run_id: None,
            initial_messages,
            max_ticks: 10_000,
        }
    }

    pub fn with_run_id(mut self, run_id: Uuid) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_max_ticks(mut self, max_ticks: usize) -> Self {
        self.max_ticks = max_ticks;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStatus {
    Completed,
    Drained,
    BudgetExceeded,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRunOutput {
    pub status: GraphRunStatus,
    pub messages: Vec<RunMessage>,
    pub state: GraphRuntimeState,
    pub ledger: GraphRunLedger,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NodeExecutionContext {
    pub run_id: Uuid,
    pub attempt_id: Uuid,
    pub node_id: NodeId,
}

pub trait NodeExecutor: Send + Sync {
    fn execute(
        &self,
        node: NodeSpec,
        input: NodeInput,
        ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>>;
}

impl<F, Fut> NodeExecutor for F
where
    F: Fn(NodeSpec, NodeInput, NodeExecutionContext) -> Fut + Send + Sync,
    Fut: Future<Output = AgentCoreResult<NodeOutput>> + Send + 'static,
{
    fn execute(
        &self,
        node: NodeSpec,
        input: NodeInput,
        ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
        Box::pin(self(node, input, ctx))
    }
}

#[derive(Clone)]
pub struct GraphRuntimeServices {
    pub executor: Arc<dyn NodeExecutor>,
}

impl GraphRuntimeServices {
    pub fn new(executor: Arc<dyn NodeExecutor>) -> Self {
        Self { executor }
    }
}

type RunningFuture = Pin<Box<dyn Future<Output = RunningResult> + Send + 'static>>;

struct RunningResult {
    attempt_id: Uuid,
    node_id: NodeId,
    package_version: u64,
    concurrency_key: Option<String>,
    output: AgentCoreResult<NodeOutput>,
}

#[derive(Default)]
struct RunningCounts {
    by_node: BTreeMap<NodeId, usize>,
    by_key: BTreeMap<(NodeId, String), usize>,
}

impl RunningCounts {
    fn admits(&self, node: &NodeSpec, activation: &NodeActivation) -> bool {
        match &node.concurrency {
            NodeConcurrency::Serial => self.by_node.get(&node.id).copied().unwrap_or_default() == 0,
            NodeConcurrency::Parallel { max } => {
                self.by_node.get(&node.id).copied().unwrap_or_default() < *max
            }
            NodeConcurrency::ByKey { key, max_per_key } => {
                let Some(value) = concurrency_value(key, &activation.input) else {
                    return self.by_node.get(&node.id).copied().unwrap_or_default() == 0;
                };
                self.by_key
                    .get(&(node.id.clone(), value))
                    .copied()
                    .unwrap_or_default()
                    < *max_per_key
            }
        }
    }

    fn start(&mut self, node: &NodeSpec, key: Option<String>) {
        *self.by_node.entry(node.id.clone()).or_default() += 1;
        if let Some(key) = key {
            *self.by_key.entry((node.id.clone(), key)).or_default() += 1;
        }
    }

    fn finish(&mut self, node_id: &str, key: Option<String>) {
        decrement(self.by_node.get_mut(node_id));
        if let Some(key) = key {
            decrement(self.by_key.get_mut(&(node_id.to_string(), key)));
        }
    }
}

fn decrement(value: Option<&mut usize>) {
    if let Some(value) = value {
        *value = value.saturating_sub(1);
    }
}

fn concurrency_value(key: &ConcurrencyKey, input: &NodeInput) -> Option<String> {
    input
        .required
        .get(&key.item)
        .or_else(|| input.optional.get(&key.item))
        .and_then(|matches| matches.first())
        .map(|matched| matched.selected.hash.to_string())
}

pub struct GraphRuntime {
    graph: GraphSpec,
    services: GraphRuntimeServices,
    state: GraphRuntimeState,
    activations: VecDeque<NodeActivation>,
    running: FuturesUnordered<RunningFuture>,
    running_counts: RunningCounts,
    ledger: GraphRunLedger,
    run_id: Uuid,
}

impl GraphRuntime {
    pub fn new(graph: GraphSpec, services: GraphRuntimeServices) -> Self {
        let mut edge_states = BTreeMap::new();
        for edge in &graph.edges {
            edge_states.insert(edge.id.clone(), EdgeState::new(edge.id.clone()));
        }

        let mut package_states = BTreeMap::new();
        for node in graph.nodes.values() {
            let package_ref = PackageRef::new(node.id.clone(), node.input.name.clone());
            package_states.insert(
                package_ref,
                PackageState::new(node.id.clone(), node.input.name.clone()),
            );
        }

        Self {
            graph,
            services,
            state: GraphRuntimeState {
                output_logs: BTreeMap::new(),
                edge_states,
                package_states,
            },
            activations: VecDeque::new(),
            running: FuturesUnordered::new(),
            running_counts: RunningCounts::default(),
            ledger: GraphRunLedger::default(),
            run_id: Uuid::new_v4(),
        }
    }

    pub async fn run(mut self, input: GraphRunInput) -> AgentCoreResult<GraphRunOutput> {
        if let Some(run_id) = input.run_id {
            self.run_id = run_id;
        }

        for message in input.initial_messages {
            let graph_input = self.graph.input.clone();
            self.commit_output(&graph_input, message);
        }

        let mut ticks = 0usize;
        loop {
            if self.finish_policy_satisfied() {
                return Ok(self.finish(GraphRunStatus::Completed, None));
            }

            if ticks >= input.max_ticks {
                return Ok(self.finish(
                    GraphRunStatus::BudgetExceeded,
                    Some("graph runtime tick budget exceeded".to_string()),
                ));
            }
            ticks += 1;

            self.scan_edges()?;
            self.enqueue_ready_activations()?;
            self.spawn_ready_activations()?;

            if self.finish_policy_satisfied() && self.running.is_empty() {
                return Ok(self.finish(GraphRunStatus::Completed, None));
            }

            if let Some(result) = self.running.next().await {
                if let Err(error) = self.commit_running_result(result) {
                    return Ok(self.finish(GraphRunStatus::Failed, Some(error.to_string())));
                }
                if self.finish_policy_satisfied() {
                    return Ok(self.finish(GraphRunStatus::Completed, None));
                }
                continue;
            }

            if self.activations.is_empty() {
                let status = if self.finish_policy_satisfied() {
                    GraphRunStatus::Completed
                } else {
                    GraphRunStatus::Drained
                };
                return Ok(self.finish(status, None));
            }
        }
    }

    fn finish(self, status: GraphRunStatus, error: Option<String>) -> GraphRunOutput {
        let messages = self
            .state
            .output_logs
            .values()
            .flat_map(|entries| entries.iter().map(|entry| entry.message.clone()))
            .collect();
        GraphRunOutput {
            status,
            messages,
            state: self.state,
            ledger: self.ledger,
            error,
        }
    }

    fn commit_output(&mut self, output: &OutputRef, mut message: RunMessage) {
        message.set_source_node_id_if_empty(output.node.clone());
        let message_version = self.next_message_version(message.id);
        let log = self.state.output_logs.entry(output.clone()).or_default();
        let seq = log.len() as u64;
        log.push(OutputLogEntry {
            seq,
            node: output.node.clone(),
            port: output.port.clone(),
            message,
            message_version,
        });
    }

    fn next_message_version(&self, message_id: Uuid) -> u64 {
        self.state
            .output_logs
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|entry| entry.message.id == message_id)
            .count() as u64
            + 1
    }

    fn scan_edges(&mut self) -> AgentCoreResult<()> {
        let edges = self.graph.edges.clone();
        for edge in edges {
            let from_seq = self
                .state
                .edge_states
                .get(&edge.id)
                .map(|state| state.next_seq)
                .unwrap_or_default();
            self.ledger.events.push(RuntimeEvent::EdgeScanned {
                edge_id: edge.id.clone(),
                from_seq,
            });

            let entries = self
                .state
                .output_logs
                .get(&edge.from)
                .cloned()
                .unwrap_or_default();

            for entry in entries.into_iter().filter(|entry| entry.seq >= from_seq) {
                let matched = self.deliver_entry(&edge, &entry)?;
                if !matched {
                    self.ledger.events.push(RuntimeEvent::MessageFiltered {
                        edge_id: edge.id.clone(),
                        seq: entry.seq,
                    });
                }
                if let Some(edge_state) = self.state.edge_states.get_mut(&edge.id) {
                    edge_state.next_seq = entry.seq + 1;
                }
            }
        }
        Ok(())
    }

    fn deliver_entry(&mut self, edge: &GraphEdge, entry: &OutputLogEntry) -> AgentCoreResult<bool> {
        let target_node = self.graph.nodes.get(&edge.to.node).ok_or_else(|| {
            AgentCoreError::InvalidConfig(format!("edge target node not found: {}", edge.to.node))
        })?;
        let package_spec = target_node.input.clone();
        let mut matched_any = false;

        for (item, _) in package_spec.items() {
            let Some(selected) = item.query.matches(&entry.message)? else {
                continue;
            };
            matched_any = true;
            let delivery_key = DeliveryKey {
                edge_id: edge.id.clone(),
                package: edge.to.package.clone(),
                item: item.name.clone(),
                message_id: entry.message.id,
                selected_hash: selected.hash,
            };

            let is_new = self
                .state
                .edge_states
                .get_mut(&edge.id)
                .map(|state| state.delivered.insert(delivery_key))
                .unwrap_or(false);
            if !is_new {
                continue;
            }

            let content = MatchedContent {
                source: edge.from.clone(),
                message_id: entry.message.id,
                message_version: entry.message_version,
                selected,
            };
            let package_state = self.state.package_states.get_mut(&edge.to).ok_or_else(|| {
                AgentCoreError::InvalidConfig(format!(
                    "target package state not found: {}.{}",
                    edge.to.node, edge.to.package
                ))
            })?;
            package_state.insert(item, content.clone());
            self.ledger.transfers.push(EdgeTransferRecord {
                edge_id: edge.id.clone(),
                from: edge.from.clone(),
                to: edge.to.clone(),
                item: item.name.clone(),
                message_id: entry.message.id,
                message_version: entry.message_version,
                selected_hash: content.selected.hash,
            });
            self.ledger.events.push(RuntimeEvent::PackageUpdated {
                node: edge.to.node.clone(),
                package: edge.to.package.clone(),
                version: package_state.version,
            });
        }

        Ok(matched_any)
    }

    fn enqueue_ready_activations(&mut self) -> AgentCoreResult<()> {
        let package_refs: Vec<_> = self.state.package_states.keys().cloned().collect();
        for package_ref in package_refs {
            let node = self.graph.nodes.get(&package_ref.node).ok_or_else(|| {
                AgentCoreError::InvalidConfig(format!("node not found: {}", package_ref.node))
            })?;
            let package_state =
                self.state
                    .package_states
                    .get_mut(&package_ref)
                    .ok_or_else(|| {
                        AgentCoreError::InvalidConfig(format!(
                            "package state not found: {}.{}",
                            package_ref.node, package_ref.package
                        ))
                    })?;
            if !package_state.is_ready(&node.input) {
                continue;
            }
            if package_state.last_activated_version == Some(package_state.version) {
                continue;
            }

            let input = NodeInput::from_package(&node.input, package_state);
            let activation = NodeActivation {
                id: Uuid::new_v4(),
                node: package_ref.node.clone(),
                package: package_ref.package.clone(),
                package_version: package_state.version,
                input,
            };
            package_state.last_activated_version = Some(package_state.version);
            self.ledger.events.push(RuntimeEvent::NodeActivated {
                node: activation.node.clone(),
                package_version: activation.package_version,
            });
            self.activations.push_back(activation);
        }
        Ok(())
    }

    fn spawn_ready_activations(&mut self) -> AgentCoreResult<()> {
        let mut remaining = VecDeque::new();
        while let Some(activation) = self.activations.pop_front() {
            let node = self.graph.nodes.get(&activation.node).ok_or_else(|| {
                AgentCoreError::InvalidConfig(format!("node not found: {}", activation.node))
            })?;
            if !self.running_counts.admits(node, &activation) {
                remaining.push_back(activation);
                continue;
            }

            let key = match &node.concurrency {
                NodeConcurrency::ByKey { key, .. } => concurrency_value(key, &activation.input),
                _ => None,
            };
            self.running_counts.start(node, key.clone());
            let attempt_id = Uuid::new_v4();
            let ctx = NodeExecutionContext {
                run_id: self.run_id,
                attempt_id,
                node_id: node.id.clone(),
            };
            let future =
                self.services
                    .executor
                    .execute(node.clone(), activation.input.clone(), ctx);
            let node_id = node.id.clone();
            let package_version = activation.package_version;
            self.running.push(Box::pin(async move {
                RunningResult {
                    attempt_id,
                    node_id,
                    package_version,
                    concurrency_key: key,
                    output: future.await,
                }
            }) as RunningFuture);
        }
        self.activations = remaining;
        Ok(())
    }

    fn commit_running_result(&mut self, result: RunningResult) -> AgentCoreResult<()> {
        self.running_counts
            .finish(&result.node_id, result.concurrency_key);
        match result.output {
            Ok(output) => {
                for (port, messages) in output.messages {
                    let output_ref = OutputRef::new(result.node_id.clone(), port);
                    for message in messages {
                        self.commit_output(&output_ref, message);
                    }
                }
                self.ledger.node_attempts.push(NodeAttemptRecord {
                    attempt_id: result.attempt_id,
                    node: result.node_id.clone(),
                    package_version: result.package_version,
                    status: NodeAttemptStatus::Completed,
                });
                self.ledger.events.push(RuntimeEvent::NodeCompleted {
                    node: result.node_id,
                });
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.ledger.node_attempts.push(NodeAttemptRecord {
                    attempt_id: result.attempt_id,
                    node: result.node_id.clone(),
                    package_version: result.package_version,
                    status: NodeAttemptStatus::Failed,
                });
                self.ledger.events.push(RuntimeEvent::NodeFailed {
                    node: result.node_id,
                    error: message.clone(),
                });
                Err(AgentCoreError::Recoverable(message))
            }
        }
    }

    fn finish_policy_satisfied(&self) -> bool {
        self.graph.finish_node.as_ref().is_some_and(|finish_node| {
            self.ledger.node_attempts.iter().any(|attempt| {
                attempt.node == *finish_node && attempt.status == NodeAttemptStatus::Completed
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use serde_json::json;
    use std::sync::Mutex;
    use std::task::{Context, Poll};

    fn text_message(text: &str) -> RunMessage {
        RunMessage::user(vec![ContentBlock::text(text)]).unwrap()
    }

    fn assistant_message(text: &str) -> RunMessage {
        RunMessage::assistant(vec![ContentBlock::text(text)]).unwrap()
    }

    #[derive(Default)]
    struct RecordingExecutor {
        calls: Mutex<Vec<NodeId>>,
    }

    impl NodeExecutor for RecordingExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            self.calls.lock().unwrap().push(node.id.clone());
            Box::pin(async move {
                let output = match node.kind {
                    NodeKind::Final => NodeOutput::new(),
                    NodeKind::Tool(spec) => NodeOutput::new().with_message(
                        spec.result_port,
                        RunMessage::tool(vec![ContentBlock::tool_result(
                            "call",
                            Some(spec.tool_name),
                            json!({"ok": true}),
                            false,
                        )])?,
                    ),
                    NodeKind::Agent(_) => {
                        if input
                            .optional
                            .get("tool_result")
                            .is_some_and(|matches| !matches.is_empty())
                        {
                            NodeOutput::new().with_message("final", assistant_message("done"))
                        } else {
                            NodeOutput::new().with_message(
                                "tool_calls",
                                RunMessage::assistant(vec![ContentBlock::tool_call(
                                    "call",
                                    "tap",
                                    json!({"x": 1}),
                                )])?,
                            )
                        }
                    }
                    _ => NodeOutput::new().with_message("out", assistant_message("ok")),
                };
                Ok(output)
            })
        }
    }

    #[test]
    fn filtered_message_does_not_update_package() {
        let graph = GraphSpec::builder("filtered")
            .node(
                NodeSpec::new(
                    "target",
                    NodeKind::Final,
                    InputPackageSpec::new("input").required(
                        "instruction",
                        MessageQuery::where_eq("metadata.kind", "instruction"),
                        Cardinality::Latest,
                    ),
                )
                .output("done"),
            )
            .edge(
                "input_to_target",
                ("input", "messages"),
                ("target", "input"),
            )
            .finish_at("target")
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![text_message("ignored")])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Drained);
        let package = output
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        assert_eq!(package.version, 0);
        assert!(output.ledger.transfers.is_empty());
    }

    #[test]
    fn any_update_is_required_latest_item() {
        let graph = GraphSpec::builder("any")
            .node(
                NodeSpec::final_node(
                    "target",
                    InputPackageSpec::new("input").required(
                        "turn",
                        MessageQuery::any(),
                        Cardinality::Latest,
                    ),
                )
                .output("done"),
            )
            .edge(
                "input_to_target",
                ("input", "messages"),
                ("target", "input"),
            )
            .finish_at("target")
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![text_message("go")])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(output.ledger.transfers.len(), 1);
        assert_eq!(output.ledger.node_attempts.len(), 1);
    }

    #[test]
    fn message_version_change_without_selected_field_change_does_not_activate_twice() {
        let mut first = RunMessage::user(vec![ContentBlock::text("same")]).unwrap();
        first.metadata.insert("ignored".to_string(), json!(1));
        let mut second = first.clone();
        second.metadata.insert("ignored".to_string(), json!(2));

        let graph = GraphSpec::builder("selected")
            .node(
                NodeSpec::final_node(
                    "target",
                    InputPackageSpec::new("input").required(
                        "text",
                        MessageQuery::where_exists("content[*].text").select(["content[*].text"]),
                        Cardinality::Latest,
                    ),
                )
                .output("done"),
            )
            .edge(
                "input_to_target",
                ("input", "messages"),
                ("target", "input"),
            )
            .finish_at("target")
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![first, second])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(output.ledger.transfers.len(), 1);
        assert_eq!(output.ledger.node_attempts.len(), 1);
    }

    #[test]
    fn package_requires_all_required_items_and_keeps_optional() {
        let instruction = RunMessage::user(vec![ContentBlock::text("do")])
            .unwrap()
            .with_metadata("kind", json!("instruction"));
        let observation = RunMessage::tool(vec![ContentBlock::tool_result(
            "call",
            Some("state".to_string()),
            json!({"screen": "home"}),
            false,
        )])
        .unwrap();

        let graph = GraphSpec::builder("package")
            .node(NodeSpec::final_node(
                "target",
                InputPackageSpec::new("input")
                    .required(
                        "instruction",
                        MessageQuery::where_eq("metadata.kind", "instruction"),
                        Cardinality::Latest,
                    )
                    .required(
                        "observation",
                        MessageQuery::where_eq("content[*].type", "tool_result"),
                        Cardinality::Latest,
                    )
                    .optional(
                        "text",
                        MessageQuery::where_exists("content[*].text"),
                        Cardinality::Latest,
                    ),
            ))
            .edge(
                "input_to_target",
                ("input", "messages"),
                ("target", "input"),
            )
            .finish_at("target")
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![instruction, observation])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Completed);
        let package = output
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        assert!(package.items.contains_key("instruction"));
        assert!(package.items.contains_key("observation"));
        assert!(package.items.contains_key("text"));
    }

    #[test]
    fn react_graph_runs_agent_tool_agent_final_without_special_loop() {
        let graph = GraphSpec::builder("react")
            .node(
                NodeSpec::agent(
                    "agent",
                    "phone_agent",
                    InputPackageSpec::new("context")
                        .required("turn", MessageQuery::any(), Cardinality::Latest)
                        .optional(
                            "tool_result",
                            MessageQuery::where_eq("content[*].type", "tool_result"),
                            Cardinality::Latest,
                        ),
                )
                .output("tool_calls")
                .output("final")
                .concurrency(NodeConcurrency::Serial),
            )
            .node(
                NodeSpec::tool(
                    "tap_tool",
                    "tap",
                    "tool_call",
                    "results",
                    InputPackageSpec::new("calls").required(
                        "tool_call",
                        MessageQuery::where_eq("content[*].type", "tool_call"),
                        Cardinality::Latest,
                    ),
                )
                .concurrency(NodeConcurrency::Parallel { max: 4 }),
            )
            .node(NodeSpec::final_node(
                "final",
                InputPackageSpec::new("answer").required(
                    "final_answer",
                    MessageQuery::where_exists("content[*].text"),
                    Cardinality::Latest,
                ),
            ))
            .edge(
                "input_to_agent",
                ("input", "messages"),
                ("agent", "context"),
            )
            .edge(
                "agent_to_tool",
                ("agent", "tool_calls"),
                ("tap_tool", "calls"),
            )
            .edge(
                "tool_to_agent",
                ("tap_tool", "results"),
                ("agent", "context"),
            )
            .edge("agent_to_final", ("agent", "final"), ("final", "answer"))
            .finish_at("final")
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![text_message("tap something")])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert!(output
            .ledger
            .node_attempts
            .iter()
            .any(|attempt| attempt.node == "agent"));
        assert!(output
            .ledger
            .node_attempts
            .iter()
            .any(|attempt| attempt.node == "tap_tool"));
        assert!(output
            .ledger
            .node_attempts
            .iter()
            .any(|attempt| attempt.node == "final"));
    }

    struct YieldOnce {
        yielded: bool,
    }

    impl YieldOnce {
        fn new() -> Self {
            Self { yielded: false }
        }
    }

    impl Future for YieldOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.yielded {
                Poll::Ready(())
            } else {
                self.yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    #[derive(Default)]
    struct YieldingExecutor;

    impl NodeExecutor for YieldingExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            _input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            Box::pin(async move {
                if node.id == "slow" {
                    YieldOnce::new().await;
                }
                Ok(NodeOutput::new())
            })
        }
    }

    #[test]
    fn slow_node_future_does_not_block_independent_ready_node() {
        let package = || {
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )
        };
        let graph = GraphSpec::builder("parallel")
            .node(NodeSpec::new(
                "slow",
                NodeKind::Transform {
                    executor: "slow".to_string(),
                    config: json!({}),
                },
                package(),
            ))
            .node(NodeSpec::new(
                "fast",
                NodeKind::Transform {
                    executor: "fast".to_string(),
                    config: json!({}),
                },
                package(),
            ))
            .edge("input_to_slow", ("input", "messages"), ("slow", "input"))
            .edge("input_to_fast", ("input", "messages"), ("fast", "input"))
            .build()
            .unwrap();

        let output = block_on(
            GraphRuntime::new(graph, GraphRuntimeServices::new(Arc::new(YieldingExecutor)))
                .run(GraphRunInput::new(vec![text_message("go")])),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::Drained);
        assert_eq!(output.ledger.node_attempts.len(), 2);
        assert_eq!(output.ledger.node_attempts[0].node, "fast");
        assert_eq!(output.ledger.node_attempts[1].node, "slow");
    }
}
