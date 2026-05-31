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
        let (message_filters, content_filters): (Vec<_>, Vec<_>) = self
            .filters
            .iter()
            .partition(|filter| !filter.path.0.starts_with("content[*]"));

        for filter in message_filters {
            let values = values_at_path(message, &filter.path.0, None)?;
            if !filter_matches(&values, &filter.op) {
                return Ok(None);
            }
        }

        let matching_content_indexes = if content_filters.is_empty() {
            None
        } else {
            let indexes = matching_content_indexes(message, &content_filters)?;
            if indexes.is_empty() {
                return Ok(None);
            }
            Some(indexes)
        };

        Ok(Some(select_fields(
            message,
            &self.select,
            matching_content_indexes.as_deref(),
        )?))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectedFields {
    pub value: Value,
    pub hash: ContentHash,
}

fn select_fields(
    message: &RunMessage,
    mask: &FieldMask,
    content_indexes: Option<&[usize]>,
) -> AgentCoreResult<SelectedFields> {
    let value = if mask.include.is_empty() {
        if let Some(indexes) = content_indexes {
            let mut value = serde_json::to_value(message)?;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "content".to_string(),
                    Value::Array(content_values(message, Some(indexes))?),
                );
            }
            value
        } else {
            serde_json::to_value(message)?
        }
    } else {
        let mut object = Map::new();
        for path in &mask.include {
            let value = if path.0 == "content[*]" {
                Value::Array(content_values(message, content_indexes)?)
            } else {
                values_to_value(values_at_path(message, &path.0, content_indexes)?)
            };
            object.insert(path.0.clone(), value);
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

fn values_at_path(
    message: &RunMessage,
    path: &str,
    content_indexes: Option<&[usize]>,
) -> AgentCoreResult<Vec<Value>> {
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
        return content_values(message, content_indexes);
    }
    if let Some(field) = path.strip_prefix("content[*].") {
        let mut values = Vec::new();
        for block in selected_content_blocks(message, content_indexes) {
            values.extend(content_block_values(block, field)?);
        }
        return Ok(values);
    }

    Err(AgentCoreError::InvalidInput(format!(
        "unsupported message field path: {path}"
    )))
}

fn selected_content_blocks<'a>(
    message: &'a RunMessage,
    content_indexes: Option<&[usize]>,
) -> Vec<&'a ContentBlock> {
    match content_indexes {
        Some(indexes) => indexes
            .iter()
            .filter_map(|index| message.content.get(*index))
            .collect(),
        None => message.content.iter().collect(),
    }
}

fn content_values(
    message: &RunMessage,
    content_indexes: Option<&[usize]>,
) -> AgentCoreResult<Vec<Value>> {
    selected_content_blocks(message, content_indexes)
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn matching_content_indexes(
    message: &RunMessage,
    filters: &[&FieldFilter],
) -> AgentCoreResult<Vec<usize>> {
    let mut indexes = Vec::new();
    for (index, block) in message.content.iter().enumerate() {
        let mut matches_all = true;
        for filter in filters {
            let values = if filter.path.0 == "content[*]" {
                vec![serde_json::to_value(block)?]
            } else if let Some(field) = filter.path.0.strip_prefix("content[*].") {
                content_block_values(block, field)?
            } else {
                Vec::new()
            };
            if !filter_matches(&values, &filter.op) {
                matches_all = false;
                break;
            }
        }
        if matches_all {
            indexes.push(index);
        }
    }
    Ok(indexes)
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
    }

    fn bump_version(&mut self) {
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
    NodeStarted {
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
    pub stop_requested: bool,
}

impl GraphRunInput {
    pub fn new(initial_messages: Vec<RunMessage>) -> Self {
        Self {
            run_id: None,
            initial_messages,
            max_ticks: 10_000,
            stop_requested: false,
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

    pub fn with_stop_requested(mut self, stop_requested: bool) -> Self {
        self.stop_requested = stop_requested;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStatus {
    Completed,
    Drained,
    Cancelled,
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

        if input.stop_requested {
            return Ok(self.finish(GraphRunStatus::Cancelled, None));
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
                    let message = error.to_string();
                    let status = if message.to_ascii_lowercase().contains("cancel") {
                        GraphRunStatus::Cancelled
                    } else {
                        GraphRunStatus::Failed
                    };
                    return Ok(self.finish(status, Some(message)));
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
        let mut inserted_any = false;

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
            inserted_any = true;
            self.ledger.transfers.push(EdgeTransferRecord {
                edge_id: edge.id.clone(),
                from: edge.from.clone(),
                to: edge.to.clone(),
                item: item.name.clone(),
                message_id: entry.message.id,
                message_version: entry.message_version,
                selected_hash: content.selected.hash,
            });
        }

        if inserted_any {
            let package_state = self.state.package_states.get_mut(&edge.to).ok_or_else(|| {
                AgentCoreError::InvalidConfig(format!(
                    "target package state not found: {}.{}",
                    edge.to.node, edge.to.package
                ))
            })?;
            package_state.bump_version();
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
            self.ledger.events.push(RuntimeEvent::NodeStarted {
                node: node.id.clone(),
                package_version: activation.package_version,
            });
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::task::{Context, Poll};

    fn text_message(text: &str) -> RunMessage {
        RunMessage::user(vec![ContentBlock::text(text)]).unwrap()
    }

    fn assistant_message(text: &str) -> RunMessage {
        RunMessage::assistant(vec![ContentBlock::text(text)]).unwrap()
    }

    fn tool_call_message(call_id: &str, tool_name: &str) -> RunMessage {
        RunMessage::assistant(vec![ContentBlock::tool_call(
            call_id,
            tool_name,
            json!({"x": 1}),
        )])
        .unwrap()
    }

    fn tool_result_message(call_id: &str, tool_name: &str, is_error: bool) -> RunMessage {
        RunMessage::tool(vec![ContentBlock::tool_result(
            call_id,
            Some(tool_name.to_string()),
            json!({"ok": !is_error}),
            is_error,
        )])
        .unwrap()
    }

    fn user_turn_query() -> MessageQuery {
        MessageQuery::where_eq("role", "user")
    }

    fn tool_result_query() -> MessageQuery {
        MessageQuery::where_eq("content[*].type", "tool_result")
    }

    fn target_graph(package: InputPackageSpec) -> GraphSpec {
        GraphSpec::builder("target")
            .node(NodeSpec::final_node("target", package))
            .edge(
                "input_to_target",
                ("input", "messages"),
                ("target", "input"),
            )
            .finish_at("target")
            .build()
            .unwrap()
    }

    fn run_with_executor(
        graph: GraphSpec,
        messages: Vec<RunMessage>,
        executor: Arc<dyn NodeExecutor>,
    ) -> GraphRunOutput {
        block_on(
            GraphRuntime::new(graph, GraphRuntimeServices::new(executor))
                .run(GraphRunInput::new(messages)),
        )
        .unwrap()
    }

    #[derive(Clone, Default)]
    struct TestExecutor {
        inputs: Arc<Mutex<Vec<(NodeId, NodeInput)>>>,
        outputs: Arc<Mutex<BTreeMap<NodeId, NodeOutput>>>,
        errors: Arc<Mutex<BTreeMap<NodeId, String>>>,
    }

    impl TestExecutor {
        fn output(self, node: &str, output: NodeOutput) -> Self {
            self.outputs
                .lock()
                .unwrap()
                .insert(node.to_string(), output);
            self
        }

        fn error(self, node: &str, message: &str) -> Self {
            self.errors
                .lock()
                .unwrap()
                .insert(node.to_string(), message.to_string());
            self
        }

        fn inputs_for(&self, node: &str) -> Vec<NodeInput> {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .filter(|(node_id, _)| node_id == node)
                .map(|(_, input)| input.clone())
                .collect()
        }
    }

    impl NodeExecutor for TestExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            self.inputs.lock().unwrap().push((node.id.clone(), input));
            let error = self.errors.lock().unwrap().get(&node.id).cloned();
            let output = self
                .outputs
                .lock()
                .unwrap()
                .get(&node.id)
                .cloned()
                .unwrap_or_default();
            Box::pin(async move {
                if let Some(error) = error {
                    Err(AgentCoreError::Recoverable(error))
                } else {
                    Ok(output)
                }
            })
        }
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
    fn edge_scans_only_unseen_entries() {
        let graph = target_graph(InputPackageSpec::new("input").required(
            "turn",
            MessageQuery::any(),
            Cardinality::AtLeast(2),
        ));
        let mut runtime = GraphRuntime::new(
            graph,
            GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
        );
        let input = runtime.graph.input.clone();
        runtime.commit_output(&input, text_message("one"));
        runtime.commit_output(&input, text_message("two"));

        runtime.scan_edges().unwrap();
        assert_eq!(runtime.ledger.transfers.len(), 2);
        assert_eq!(
            runtime
                .state
                .edge_states
                .get("input_to_target")
                .unwrap()
                .next_seq,
            2
        );

        runtime.scan_edges().unwrap();
        assert_eq!(runtime.ledger.transfers.len(), 2);
        assert!(runtime.ledger.events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::EdgeScanned {
                    edge_id,
                    from_seq: 2
                } if edge_id == "input_to_target"
            )
        }));
    }

    #[test]
    fn edge_ignores_unmatched_message() {
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "instruction",
                MessageQuery::where_eq("metadata.kind", "instruction"),
                Cardinality::Latest,
            )),
            vec![text_message("plain")],
            Arc::new(RecordingExecutor::default()),
        );

        let package = output
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        assert_eq!(package.version, 0);
        assert!(output.ledger.transfers.is_empty());
    }

    #[test]
    fn edge_delivers_matched_content_once() {
        let mut first = text_message("same");
        first.metadata.insert("ignored".to_string(), json!(1));
        let mut second = first.clone();
        second.metadata.insert("ignored".to_string(), json!(2));
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "text",
                MessageQuery::where_exists("content[*].text").select(["content[*].text"]),
                Cardinality::Latest,
            )),
            vec![first, second],
            Arc::new(RecordingExecutor::default()),
        );

        assert_eq!(output.ledger.transfers.len(), 1);
        assert_eq!(
            output
                .state
                .package_states
                .get(&PackageRef::new("target", "input"))
                .unwrap()
                .version,
            1
        );
    }

    #[test]
    fn same_message_version_changed_but_selected_unchanged_no_activation() {
        message_version_change_without_selected_field_change_does_not_activate_twice();
    }

    #[test]
    fn same_message_selected_field_changed_triggers_activation() {
        let first = text_message("old");
        let mut second = first.clone();
        second.content = vec![ContentBlock::text("new")];

        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "text",
                MessageQuery::where_exists("content[*].text").select(["content[*].text"]),
                Cardinality::Latest,
            )),
            vec![first, second],
            Arc::new(RecordingExecutor::default()),
        );

        assert_eq!(output.ledger.transfers.len(), 2);
        assert_eq!(output.ledger.node_attempts.len(), 1);
        let package = output
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        assert_eq!(package.version, 2);
    }

    #[test]
    fn different_message_same_selected_hash_still_delivered() {
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "text",
                MessageQuery::where_exists("content[*].text").select(["content[*].text"]),
                Cardinality::Latest,
            )),
            vec![text_message("same"), text_message("same")],
            Arc::new(RecordingExecutor::default()),
        );

        assert_eq!(output.ledger.transfers.len(), 2);
        assert_ne!(
            output.ledger.transfers[0].message_id,
            output.ledger.transfers[1].message_id
        );
    }

    #[test]
    fn required_item_latest_materializes_only_latest() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "text",
                MessageQuery::where_exists("content[*].text").select(["content[*].text"]),
                Cardinality::Latest,
            )),
            vec![text_message("old"), text_message("new")],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        let matches = input.required.get("text").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].selected.value["content[*].text"], json!("new"));
    }

    #[test]
    fn required_item_at_least_waits_until_enough_matches() {
        let graph = target_graph(InputPackageSpec::new("input").required(
            "turns",
            MessageQuery::any(),
            Cardinality::AtLeast(2),
        ));

        let one = run_with_executor(
            graph.clone(),
            vec![text_message("one")],
            Arc::new(RecordingExecutor::default()),
        );
        assert_eq!(one.status, GraphRunStatus::Drained);
        assert!(one.ledger.node_attempts.is_empty());

        let two = run_with_executor(
            graph,
            vec![text_message("one"), text_message("two")],
            Arc::new(RecordingExecutor::default()),
        );
        assert_eq!(two.status, GraphRunStatus::Completed);
        assert_eq!(two.ledger.node_attempts.len(), 1);
    }

    #[test]
    fn multiple_required_items_all_must_be_ready() {
        let graph = target_graph(
            InputPackageSpec::new("input")
                .required(
                    "task",
                    MessageQuery::where_eq("metadata.kind", "task"),
                    Cardinality::Latest,
                )
                .required(
                    "dependency",
                    MessageQuery::where_eq("metadata.kind", "dependency"),
                    Cardinality::Latest,
                ),
        );
        let task = text_message("task").with_metadata("kind", json!("task"));
        let dependency = text_message("dep").with_metadata("kind", json!("dependency"));

        let missing = run_with_executor(
            graph.clone(),
            vec![task.clone()],
            Arc::new(RecordingExecutor::default()),
        );
        assert_eq!(missing.status, GraphRunStatus::Drained);

        let ready = run_with_executor(
            graph,
            vec![task, dependency],
            Arc::new(RecordingExecutor::default()),
        );
        assert_eq!(ready.status, GraphRunStatus::Completed);
    }

    #[test]
    fn optional_item_does_not_block_ready() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(
                InputPackageSpec::new("input")
                    .required("turn", user_turn_query(), Cardinality::Latest)
                    .optional("tool_result", tool_result_query(), Cardinality::Latest),
            ),
            vec![text_message("go")],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        assert!(input.optional.get("tool_result").unwrap().is_empty());
    }

    #[test]
    fn optional_item_included_when_available() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(
                InputPackageSpec::new("input")
                    .required("turn", user_turn_query(), Cardinality::Latest)
                    .optional("tool_result", tool_result_query(), Cardinality::Latest),
            ),
            vec![
                text_message("go"),
                tool_result_message("call", "tap", false),
            ],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        assert_eq!(input.optional.get("tool_result").unwrap().len(), 1);
    }

    #[test]
    fn filtered_content_does_not_increment_package_version() {
        edge_ignores_unmatched_message();
    }

    #[test]
    fn package_version_changes_once_per_new_matched_content_batch() {
        let message = text_message("task").with_metadata("kind", json!("task"));
        let output = run_with_executor(
            target_graph(
                InputPackageSpec::new("input")
                    .required(
                        "by_kind",
                        MessageQuery::where_eq("metadata.kind", "task"),
                        Cardinality::Latest,
                    )
                    .required(
                        "by_text",
                        MessageQuery::where_exists("content[*].text"),
                        Cardinality::Latest,
                    ),
            ),
            vec![message],
            Arc::new(RecordingExecutor::default()),
        );

        assert_eq!(output.ledger.transfers.len(), 2);
        assert_eq!(
            output
                .state
                .package_states
                .get(&PackageRef::new("target", "input"))
                .unwrap()
                .version,
            1
        );
    }

    #[test]
    fn agent_turn_query_does_not_match_tool_result() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(
                InputPackageSpec::new("input")
                    .required("turn", user_turn_query(), Cardinality::Latest)
                    .optional("tool_result", tool_result_query(), Cardinality::Latest),
            ),
            vec![
                text_message("user task"),
                tool_result_message("call", "tap", false),
            ],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        let turn = input.required.get("turn").unwrap().first().unwrap();
        assert_eq!(turn.selected.value["role"], json!("user"));
        assert_eq!(input.optional.get("tool_result").unwrap().len(), 1);
    }

    #[test]
    fn query_select_masks_unselected_fields_from_node_input() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(
                InputPackageSpec::new("input").required(
                    "call",
                    MessageQuery::where_eq("content[*].type", "tool_call")
                        .select(["id", "content[*].tool_name"]),
                    Cardinality::Latest,
                ),
            ),
            vec![tool_call_message("call", "tap")],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        let selected = &input.required.get("call").unwrap()[0].selected.value;
        assert!(selected.get("id").is_some());
        assert_eq!(selected["content[*].tool_name"], json!("tap"));
        assert!(selected.get("content[*].arguments").is_none());
    }

    #[test]
    fn query_path_content_array_matches_nested_block() {
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "call",
                MessageQuery::where_eq("content[*].type", "tool_call"),
                Cardinality::Latest,
            )),
            vec![RunMessage::assistant(vec![
                ContentBlock::text("before"),
                ContentBlock::tool_call("call", "tap", json!({})),
            ])
            .unwrap()],
            Arc::new(RecordingExecutor::default()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(output.ledger.transfers.len(), 1);
    }

    #[test]
    fn query_multiple_blocks_selects_only_matching_blocks() {
        let executor = TestExecutor::default();
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "call",
                MessageQuery::where_eq("content[*].type", "tool_call").select(["content[*]"]),
                Cardinality::Latest,
            )),
            vec![RunMessage::assistant(vec![
                ContentBlock::text("do not inherit"),
                ContentBlock::tool_call("call", "tap", json!({})),
            ])
            .unwrap()],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let input = executor.inputs_for("target").pop().unwrap();
        let selected = &input.required.get("call").unwrap()[0].selected.value["content[*]"];
        let blocks = selected.as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], json!("tool_call"));
    }

    #[test]
    fn react_graph_runs_agent_tool_agent_final_without_special_loop() {
        let graph = GraphSpec::builder("react")
            .node(
                NodeSpec::agent(
                    "agent",
                    "phone_agent",
                    InputPackageSpec::new("context")
                        .required(
                            "turn",
                            MessageQuery::where_eq("role", "user"),
                            Cardinality::Latest,
                        )
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

    #[test]
    fn ready_package_creates_one_activation_per_version() {
        let graph = target_graph(InputPackageSpec::new("input").required(
            "turn",
            MessageQuery::any(),
            Cardinality::Latest,
        ));
        let mut runtime = GraphRuntime::new(
            graph,
            GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
        );
        let input = runtime.graph.input.clone();
        runtime.commit_output(&input, text_message("go"));
        runtime.scan_edges().unwrap();
        runtime.enqueue_ready_activations().unwrap();
        runtime.enqueue_ready_activations().unwrap();

        assert_eq!(runtime.activations.len(), 1);
    }

    #[test]
    fn new_package_version_creates_new_activation() {
        let graph = target_graph(InputPackageSpec::new("input").required(
            "turn",
            MessageQuery::any(),
            Cardinality::Latest,
        ));
        let mut runtime = GraphRuntime::new(
            graph,
            GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
        );
        let input = runtime.graph.input.clone();
        runtime.commit_output(&input, text_message("one"));
        runtime.scan_edges().unwrap();
        runtime.enqueue_ready_activations().unwrap();
        runtime.commit_output(&input, text_message("two"));
        runtime.scan_edges().unwrap();
        runtime.enqueue_ready_activations().unwrap();

        assert_eq!(runtime.activations.len(), 2);
        assert_eq!(runtime.activations[0].package_version, 1);
        assert_eq!(runtime.activations[1].package_version, 2);
    }

    #[test]
    fn node_not_activated_when_package_ready_but_version_unchanged() {
        ready_package_creates_one_activation_per_version();
    }

    #[derive(Default)]
    struct ConcurrencyProbe {
        active: AtomicUsize,
        max_seen: AtomicUsize,
    }

    impl ConcurrencyProbe {
        fn start(&self) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let mut max = self.max_seen.load(Ordering::SeqCst);
            while active > max {
                match self.max_seen.compare_exchange(
                    max,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(next) => max = next,
                }
            }
        }

        fn finish(&self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct TrackedFuture {
        probe: Arc<ConcurrencyProbe>,
        started: bool,
        yielded: bool,
    }

    impl Future for TrackedFuture {
        type Output = AgentCoreResult<NodeOutput>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if !self.started {
                self.probe.start();
                self.started = true;
            }
            if !self.yielded {
                self.yielded = true;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.probe.finish();
            Poll::Ready(Ok(NodeOutput::new()))
        }
    }

    #[derive(Clone)]
    struct ConcurrencyExecutor {
        source_outputs: Arc<BTreeMap<NodeId, RunMessage>>,
        probe: Arc<ConcurrencyProbe>,
    }

    impl NodeExecutor for ConcurrencyExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            _input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            if let Some(message) = self.source_outputs.get(&node.id).cloned() {
                return Box::pin(async move { Ok(NodeOutput::new().with_message("out", message)) });
            }
            Box::pin(TrackedFuture {
                probe: Arc::clone(&self.probe),
                started: false,
                yielded: false,
            })
        }
    }

    fn concurrency_graph(target_concurrency: NodeConcurrency, keys: &[&str]) -> GraphSpec {
        let mut builder = GraphSpec::builder("concurrency").node(
            NodeSpec::new(
                "target",
                NodeKind::Transform {
                    executor: "target".to_string(),
                    config: json!({}),
                },
                InputPackageSpec::new("input").required(
                    "key",
                    MessageQuery::where_exists("metadata.key").select(["metadata.key"]),
                    Cardinality::Latest,
                ),
            )
            .concurrency(target_concurrency),
        );
        for index in 0..keys.len() {
            let source = format!("source_{index}");
            builder = builder
                .node(
                    NodeSpec::new(
                        source.clone(),
                        NodeKind::Transform {
                            executor: "source".to_string(),
                            config: json!({}),
                        },
                        InputPackageSpec::new("input").required(
                            "turn",
                            MessageQuery::any(),
                            Cardinality::Latest,
                        ),
                    )
                    .output("out"),
                )
                .edge(
                    format!("input_to_{source}"),
                    ("input", "messages"),
                    (source.clone(), "input"),
                )
                .edge(
                    format!("{source}_to_target"),
                    (source, "out"),
                    ("target".to_string(), "input"),
                );
        }
        builder.build().unwrap()
    }

    fn run_concurrency(
        target_concurrency: NodeConcurrency,
        keys: &[&str],
    ) -> Arc<ConcurrencyProbe> {
        let graph = concurrency_graph(target_concurrency, keys);
        let source_outputs = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                (
                    format!("source_{index}"),
                    assistant_message("source").with_metadata("key", json!(key)),
                )
            })
            .collect();
        let probe = Arc::new(ConcurrencyProbe::default());
        let executor = ConcurrencyExecutor {
            source_outputs: Arc::new(source_outputs),
            probe: Arc::clone(&probe),
        };
        let output = run_with_executor(graph, vec![text_message("go")], Arc::new(executor));
        assert_eq!(output.status, GraphRunStatus::Drained);
        probe
    }

    #[test]
    fn serial_node_prevents_concurrent_activations() {
        let probe = run_concurrency(NodeConcurrency::Serial, &["a", "b", "c"]);
        assert_eq!(probe.max_seen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parallel_node_allows_up_to_max() {
        let probe = run_concurrency(NodeConcurrency::Parallel { max: 3 }, &["a", "b", "c", "d"]);
        assert_eq!(probe.max_seen.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn by_key_limits_concurrency_per_key() {
        let probe = run_concurrency(
            NodeConcurrency::ByKey {
                key: ConcurrencyKey::package_item("key"),
                max_per_key: 1,
            },
            &["same", "same", "other"],
        );
        assert_eq!(probe.max_seen.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn runtime_finishes_when_final_node_commits_output() {
        let executor = TestExecutor::default().output(
            "target",
            NodeOutput::new().with_message("done", assistant_message("done")),
        );
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )),
            vec![text_message("go")],
            Arc::new(executor),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert!(output
            .messages
            .iter()
            .any(|message| message.source_node_id.as_deref() == Some("target")));
    }

    #[test]
    fn runtime_does_not_finish_when_final_package_ready_but_node_not_run() {
        let graph = GraphSpec::builder("blocked_final")
            .node(
                NodeSpec::final_node(
                    "final",
                    InputPackageSpec::new("input").required(
                        "turn",
                        MessageQuery::any(),
                        Cardinality::Latest,
                    ),
                )
                .concurrency(NodeConcurrency::Parallel { max: 0 }),
            )
            .edge("input_to_final", ("input", "messages"), ("final", "input"))
            .finish_at("final")
            .build()
            .unwrap();
        let output = block_on(
            GraphRuntime::new(
                graph,
                GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
            )
            .run(GraphRunInput::new(vec![text_message("go")]).with_max_ticks(2)),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::BudgetExceeded);
        assert!(output.ledger.node_attempts.is_empty());
    }

    #[test]
    fn runtime_returns_no_progress_when_no_running_no_activation_no_transfer() {
        edge_ignores_unmatched_message();
    }

    #[test]
    fn slow_node_does_not_block_independent_ready_node() {
        slow_node_future_does_not_block_independent_ready_node();
    }

    #[test]
    fn edge_scan_continues_after_node_output_commit() {
        let graph = GraphSpec::builder("chain")
            .node(
                NodeSpec::new(
                    "first",
                    NodeKind::Transform {
                        executor: "first".to_string(),
                        config: json!({}),
                    },
                    InputPackageSpec::new("input").required(
                        "turn",
                        MessageQuery::any(),
                        Cardinality::Latest,
                    ),
                )
                .output("out"),
            )
            .node(
                NodeSpec::new(
                    "second",
                    NodeKind::Transform {
                        executor: "second".to_string(),
                        config: json!({}),
                    },
                    InputPackageSpec::new("input").required(
                        "first",
                        MessageQuery::where_exists("content[*].text"),
                        Cardinality::Latest,
                    ),
                )
                .output("out"),
            )
            .node(NodeSpec::final_node(
                "final",
                InputPackageSpec::new("input").required(
                    "second",
                    MessageQuery::where_exists("content[*].text"),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_first", ("input", "messages"), ("first", "input"))
            .edge("first_to_second", ("first", "out"), ("second", "input"))
            .edge("second_to_final", ("second", "out"), ("final", "input"))
            .finish_at("final")
            .build()
            .unwrap();
        let executor = TestExecutor::default()
            .output(
                "first",
                NodeOutput::new().with_message("out", assistant_message("first")),
            )
            .output(
                "second",
                NodeOutput::new().with_message("out", assistant_message("second")),
            );

        let output = run_with_executor(graph, vec![text_message("go")], Arc::new(executor));
        let attempts: Vec<_> = output
            .ledger
            .node_attempts
            .iter()
            .map(|attempt| attempt.node.as_str())
            .collect();
        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(attempts, vec!["first", "second", "final"]);
    }

    fn react_graph() -> GraphSpec {
        GraphSpec::builder("react")
            .node(
                NodeSpec::agent(
                    "agent",
                    "phone_agent",
                    InputPackageSpec::new("context")
                        .required("turn", user_turn_query(), Cardinality::Latest)
                        .optional("tool_result", tool_result_query(), Cardinality::Latest),
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
                        MessageQuery::where_eq("content[*].type", "tool_call")
                            .select(["content[*].call_id", "content[*].tool_name"]),
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
            .unwrap()
    }

    #[derive(Clone, Copy)]
    enum ReactMode {
        Normal,
        ToolOnly,
        FinalOnly,
        InfiniteToolLoop,
        ToolError,
    }

    #[derive(Clone)]
    struct ReactExecutor {
        mode: ReactMode,
        agent_inputs: Arc<Mutex<Vec<NodeInput>>>,
    }

    impl ReactExecutor {
        fn new(mode: ReactMode) -> Self {
            Self {
                mode,
                agent_inputs: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl NodeExecutor for ReactExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            let mode = self.mode;
            if node.id == "agent" {
                self.agent_inputs.lock().unwrap().push(input.clone());
            }
            Box::pin(async move {
                match node.kind {
                    NodeKind::Agent(_) => match mode {
                        ReactMode::FinalOnly => Ok(NodeOutput::new()
                            .with_message("final", assistant_message("final answer"))),
                        ReactMode::InfiniteToolLoop => Ok(NodeOutput::new()
                            .with_message("tool_calls", tool_call_message("loop-call", "tap"))),
                        ReactMode::ToolOnly => Ok(NodeOutput::new()
                            .with_message("tool_calls", tool_call_message("call", "tap"))),
                        ReactMode::Normal | ReactMode::ToolError => {
                            if input
                                .optional
                                .get("tool_result")
                                .is_some_and(|matches| !matches.is_empty())
                            {
                                Ok(NodeOutput::new()
                                    .with_message("final", assistant_message("final answer")))
                            } else {
                                Ok(NodeOutput::new()
                                    .with_message("tool_calls", tool_call_message("call", "tap")))
                            }
                        }
                    },
                    NodeKind::Tool(spec) => {
                        let is_error = matches!(mode, ReactMode::ToolError);
                        Ok(NodeOutput::new().with_message(
                            spec.result_port,
                            tool_result_message("call", &spec.tool_name, is_error),
                        ))
                    }
                    NodeKind::Final => Ok(NodeOutput::new()),
                    _ => Ok(NodeOutput::new()),
                }
            })
        }
    }

    #[test]
    fn react_runs_agent_tool_agent_final_without_special_loop() {
        let executor = ReactExecutor::new(ReactMode::Normal);
        let output = run_with_executor(
            react_graph(),
            vec![text_message("tap something")],
            Arc::new(executor),
        );
        let attempts: Vec<_> = output
            .ledger
            .node_attempts
            .iter()
            .map(|attempt| attempt.node.as_str())
            .collect();

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(attempts, vec!["agent", "tap_tool", "agent", "final"]);
    }

    #[test]
    fn agent_tool_call_routes_only_to_tool_node() {
        let graph = GraphSpec::builder("tool_route")
            .node(
                NodeSpec::agent(
                    "agent",
                    "phone_agent",
                    InputPackageSpec::new("context").required(
                        "turn",
                        user_turn_query(),
                        Cardinality::Latest,
                    ),
                )
                .output("tool_calls")
                .output("final"),
            )
            .node(NodeSpec::tool(
                "tap_tool",
                "tap",
                "tool_call",
                "results",
                InputPackageSpec::new("calls").required(
                    "tool_call",
                    MessageQuery::where_eq("content[*].type", "tool_call"),
                    Cardinality::Latest,
                ),
            ))
            .node(NodeSpec::final_node(
                "final",
                InputPackageSpec::new("answer").required(
                    "answer",
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
            .edge("agent_to_final", ("agent", "final"), ("final", "answer"))
            .build()
            .unwrap();
        let output = run_with_executor(
            graph,
            vec![text_message("go")],
            Arc::new(ReactExecutor::new(ReactMode::ToolOnly)),
        );
        let attempts: Vec<_> = output
            .ledger
            .node_attempts
            .iter()
            .map(|attempt| attempt.node.as_str())
            .collect();
        assert!(attempts.contains(&"tap_tool"));
        assert!(!attempts.contains(&"final"));
    }

    #[test]
    fn agent_final_routes_only_to_final_node() {
        let output = run_with_executor(
            react_graph(),
            vec![text_message("go")],
            Arc::new(ReactExecutor::new(ReactMode::FinalOnly)),
        );
        let attempts: Vec<_> = output
            .ledger
            .node_attempts
            .iter()
            .map(|attempt| attempt.node.as_str())
            .collect();
        assert!(attempts.contains(&"final"));
        assert!(!attempts.contains(&"tap_tool"));
    }

    #[test]
    fn tool_result_routes_back_to_agent_context() {
        let executor = ReactExecutor::new(ReactMode::Normal);
        let output = run_with_executor(
            react_graph(),
            vec![text_message("go")],
            Arc::new(executor.clone()),
        );
        assert_eq!(output.status, GraphRunStatus::Completed);
        let inputs = executor.agent_inputs.lock().unwrap();
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[1].optional.get("tool_result").unwrap().len(), 1);
    }

    #[test]
    fn react_stops_on_final_even_if_previous_tool_results_exist() {
        let executor = ReactExecutor::new(ReactMode::Normal);
        let output = run_with_executor(
            react_graph(),
            vec![
                text_message("go"),
                tool_result_message("previous", "tap", false),
            ],
            Arc::new(executor.clone()),
        );
        let attempts: Vec<_> = output
            .ledger
            .node_attempts
            .iter()
            .map(|attempt| attempt.node.as_str())
            .collect();

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert_eq!(attempts, vec!["agent", "final"]);
    }

    #[test]
    fn react_budget_stops_infinite_tool_loop() {
        let output = block_on(
            GraphRuntime::new(
                react_graph(),
                GraphRuntimeServices::new(Arc::new(ReactExecutor::new(
                    ReactMode::InfiniteToolLoop,
                ))),
            )
            .run(GraphRunInput::new(vec![text_message("go")]).with_max_ticks(6)),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::BudgetExceeded);
    }

    #[derive(Clone)]
    struct ToolDispatchExecutor {
        fail_tool: bool,
    }

    impl NodeExecutor for ToolDispatchExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            let fail_tool = self.fail_tool;
            Box::pin(async move {
                match node.kind {
                    NodeKind::Tool(spec) => {
                        let call = input
                            .required
                            .get(&spec.call_item)
                            .unwrap()
                            .first()
                            .unwrap();
                        let selected = &call.selected.value;
                        let call_id = selected["content[*].call_id"].as_str().unwrap();
                        Ok(NodeOutput::new().with_message(
                            spec.result_port,
                            tool_result_message(call_id, &spec.tool_name, fail_tool),
                        ))
                    }
                    NodeKind::Final => Ok(NodeOutput::new()),
                    _ => Ok(NodeOutput::new()),
                }
            })
        }
    }

    fn tool_dispatch_graph() -> GraphSpec {
        GraphSpec::builder("tool_dispatch")
            .node(NodeSpec::tool(
                "tap_tool",
                "tap",
                "tool_call",
                "results",
                InputPackageSpec::new("calls").required(
                    "tool_call",
                    MessageQuery::where_eq("content[*].type", "tool_call")
                        .select(["content[*].call_id", "content[*].tool_name"]),
                    Cardinality::Latest,
                ),
            ))
            .node(NodeSpec::final_node(
                "final",
                InputPackageSpec::new("result").required(
                    "tool_result",
                    tool_result_query(),
                    Cardinality::Latest,
                ),
            ))
            .edge(
                "input_to_tool",
                ("input", "messages"),
                ("tap_tool", "calls"),
            )
            .edge(
                "tool_to_final",
                ("tap_tool", "results"),
                ("final", "result"),
            )
            .finish_at("final")
            .build()
            .unwrap()
    }

    #[test]
    fn tool_dispatcher_reads_call_item_and_emits_results_port() {
        let output = run_with_executor(
            tool_dispatch_graph(),
            vec![tool_call_message("call-123", "tap")],
            Arc::new(ToolDispatchExecutor { fail_tool: false }),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert!(output
            .state
            .output_logs
            .contains_key(&OutputRef::new("tap_tool", "results")));
    }

    #[test]
    fn tool_node_preserves_call_id_in_result() {
        let output = run_with_executor(
            tool_dispatch_graph(),
            vec![tool_call_message("call-123", "tap")],
            Arc::new(ToolDispatchExecutor { fail_tool: false }),
        );
        let result = &output
            .state
            .output_logs
            .get(&OutputRef::new("tap_tool", "results"))
            .unwrap()[0]
            .message
            .content[0];

        assert!(matches!(
            result,
            ContentBlock::ToolResult { call_id, .. } if call_id == "call-123"
        ));
    }

    #[test]
    fn tool_error_still_emits_tool_result_message() {
        let output = run_with_executor(
            tool_dispatch_graph(),
            vec![tool_call_message("call-err", "tap")],
            Arc::new(ToolDispatchExecutor { fail_tool: true }),
        );
        let result = &output
            .state
            .output_logs
            .get(&OutputRef::new("tap_tool", "results"))
            .unwrap()[0]
            .message
            .content[0];

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert!(matches!(
            result,
            ContentBlock::ToolResult {
                call_id,
                is_error: true,
                ..
            } if call_id == "call-err"
        ));
    }

    #[test]
    fn agent_node_receives_package_snapshot_not_live_state() {
        let executor = ReactExecutor::new(ReactMode::Normal);
        let output = run_with_executor(
            react_graph(),
            vec![text_message("go")],
            Arc::new(executor.clone()),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        let inputs = executor.agent_inputs.lock().unwrap();
        assert_eq!(inputs[0].version, 1);
        assert!(inputs[0].optional.get("tool_result").unwrap().is_empty());
        assert_eq!(inputs[1].version, 2);
        assert_eq!(inputs[1].optional.get("tool_result").unwrap().len(), 1);
    }

    #[test]
    fn cancel_running_node_marks_graph_cancelled() {
        let executor = TestExecutor::default().error("target", "cancelled by caller");
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )),
            vec![text_message("go")],
            Arc::new(executor),
        );

        assert_eq!(output.status, GraphRunStatus::Cancelled);
        assert!(output
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cancel"));
    }

    #[test]
    fn deadline_exceeded_stops_new_activation() {
        let graph = GraphSpec::builder("deadline")
            .node(
                NodeSpec::new(
                    "first",
                    NodeKind::Transform {
                        executor: "first".to_string(),
                        config: json!({}),
                    },
                    InputPackageSpec::new("input").required(
                        "turn",
                        MessageQuery::any(),
                        Cardinality::Latest,
                    ),
                )
                .output("out"),
            )
            .node(NodeSpec::final_node(
                "second",
                InputPackageSpec::new("input").required(
                    "first",
                    MessageQuery::where_exists("content[*].text"),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_first", ("input", "messages"), ("first", "input"))
            .edge("first_to_second", ("first", "out"), ("second", "input"))
            .finish_at("second")
            .build()
            .unwrap();
        let executor = TestExecutor::default().output(
            "first",
            NodeOutput::new().with_message("out", assistant_message("first")),
        );
        let output = block_on(
            GraphRuntime::new(graph, GraphRuntimeServices::new(Arc::new(executor)))
                .run(GraphRunInput::new(vec![text_message("go")]).with_max_ticks(1)),
        )
        .unwrap();

        assert_eq!(output.status, GraphRunStatus::BudgetExceeded);
        assert!(output
            .ledger
            .node_attempts
            .iter()
            .all(|attempt| attempt.node != "second"));
    }

    #[test]
    fn node_executor_error_records_attempt_and_policy_decides_status() {
        let executor = TestExecutor::default().error("target", "boom");
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )),
            vec![text_message("go")],
            Arc::new(executor),
        );

        assert_eq!(output.status, GraphRunStatus::Failed);
        assert_eq!(output.ledger.node_attempts.len(), 1);
        assert_eq!(
            output.ledger.node_attempts[0].status,
            NodeAttemptStatus::Failed
        );
    }

    #[test]
    fn recoverable_tool_error_can_continue_react() {
        let output = run_with_executor(
            react_graph(),
            vec![text_message("go")],
            Arc::new(ReactExecutor::new(ReactMode::ToolError)),
        );

        assert_eq!(output.status, GraphRunStatus::Completed);
        assert!(output
            .state
            .output_logs
            .get(&OutputRef::new("tap_tool", "results"))
            .unwrap()
            .iter()
            .any(|entry| matches!(
                entry.message.content.first(),
                Some(ContentBlock::ToolResult { is_error: true, .. })
            )));
    }

    #[test]
    fn ledger_records_edge_transfer() {
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )),
            vec![text_message("go")],
            Arc::new(RecordingExecutor::default()),
        );
        let transfer = output.ledger.transfers.first().unwrap();
        assert_eq!(transfer.edge_id, "input_to_target");
        assert_eq!(transfer.from, OutputRef::new("input", "messages"));
        assert_eq!(transfer.to, PackageRef::new("target", "input"));
        assert_eq!(transfer.item, "turn");
        assert_ne!(transfer.selected_hash, 0);
    }

    #[test]
    fn ledger_records_node_attempt_start_finish_status() {
        let output = run_with_executor(
            target_graph(InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            )),
            vec![text_message("go")],
            Arc::new(RecordingExecutor::default()),
        );
        assert!(output.ledger.events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::NodeStarted {
                    node,
                    package_version: 1
                } if node == "target"
            )
        }));
        assert_eq!(output.ledger.node_attempts[0].node, "target");
        assert_eq!(output.ledger.node_attempts[0].package_version, 1);
        assert_eq!(
            output.ledger.node_attempts[0].status,
            NodeAttemptStatus::Completed
        );
    }

    #[test]
    fn replay_from_logs_reconstructs_package_state() {
        let graph = target_graph(InputPackageSpec::new("input").required(
            "turn",
            MessageQuery::any(),
            Cardinality::AtLeast(2),
        ));
        let output = run_with_executor(
            graph.clone(),
            vec![text_message("one"), text_message("two")],
            Arc::new(RecordingExecutor::default()),
        );
        let mut replay = GraphRuntime::new(
            graph,
            GraphRuntimeServices::new(Arc::new(RecordingExecutor::default())),
        );
        replay.state.output_logs = output.state.output_logs.clone();
        replay.scan_edges().unwrap();

        let original = output
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        let reconstructed = replay
            .state
            .package_states
            .get(&PackageRef::new("target", "input"))
            .unwrap();
        assert_eq!(reconstructed.version, original.version);
        assert_eq!(
            reconstructed.items["turn"].matches.len(),
            original.items["turn"].matches.len()
        );
    }

    #[test]
    fn deterministic_scan_order_produces_stable_activation_order() {
        let graph = GraphSpec::builder("stable")
            .node(NodeSpec::final_node(
                "a",
                InputPackageSpec::new("input").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .node(NodeSpec::final_node(
                "b",
                InputPackageSpec::new("input").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_a", ("input", "messages"), ("a", "input"))
            .edge("input_to_b", ("input", "messages"), ("b", "input"))
            .build()
            .unwrap();
        let run_once = || {
            run_with_executor(
                graph.clone(),
                vec![text_message("go")],
                Arc::new(RecordingExecutor::default()),
            )
            .ledger
            .node_attempts
            .into_iter()
            .map(|attempt| attempt.node)
            .collect::<Vec<_>>()
        };

        assert_eq!(run_once(), run_once());
    }
}
