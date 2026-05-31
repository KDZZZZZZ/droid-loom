use agent_core::tool_executor::ToolCall;
use agent_core::{AgentCoreError, AgentCoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiElementSummary {
    pub stable_id: String,
    pub text: String,
    pub description: String,
    pub class_name: String,
    pub clickable: bool,
    pub editable: bool,
    pub dangerous: bool,
    pub bounds: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageNode {
    pub id: String,
    pub package: String,
    pub summary: String,
    pub fingerprint: String,
    pub key_buttons: Vec<UiElementSummary>,
    pub input_fields: Vec<UiElementSummary>,
    pub navigation_entries: Vec<UiElementSummary>,
    pub dangerous_actions: Vec<UiElementSummary>,
    pub visits: usize,
    pub stale: bool,
    pub last_seen_step: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateActionKind {
    ClickText,
    TypeText,
    OpenApp,
    SystemBack,
    Swipe,
    Wait,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAction {
    pub id: String,
    pub label: String,
    pub kind: CandidateActionKind,
    pub tool_name: String,
    pub arguments: Value,
    pub from_page: String,
    pub expected_target: Option<String>,
    pub read_only: bool,
    pub dangerous: bool,
}

impl CandidateAction {
    pub fn to_tool_call(&self, call_id: impl Into<String>) -> ToolCall {
        ToolCall::new(call_id, self.tool_name.clone(), self.arguments.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageTransition {
    pub from_page: String,
    pub to_page: String,
    pub action: CandidateAction,
    pub success_count: usize,
    pub failure_count: usize,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapUpdate {
    pub page_id: String,
    pub created: bool,
    pub replaced_stale_node: bool,
    pub candidate_actions: Vec<CandidateAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMapView {
    pub current_page: Option<String>,
    pub nodes: Vec<PageNode>,
    pub transitions: Vec<PageTransition>,
    pub candidate_actions: Vec<CandidateAction>,
    pub omitted_node_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub page_id: String,
    pub score: usize,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForgetScope {
    Page { page_id: String },
    Query { query: String },
    Stale,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMapMemory {
    pages: BTreeMap<String, PageNode>,
    transitions: Vec<PageTransition>,
    current_page: Option<String>,
    step_counter: usize,
}

impl AppMapMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_page(&self) -> Option<&str> {
        self.current_page.as_deref()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn transition_count(&self) -> usize {
        self.transitions.len()
    }

    pub fn pages(&self) -> impl Iterator<Item = &PageNode> {
        self.pages.values()
    }

    pub fn transitions(&self) -> &[PageTransition] {
        &self.transitions
    }

    pub fn observe_ui_state(
        &mut self,
        ui_state: &Value,
        task_hint: Option<&str>,
    ) -> AgentCoreResult<MapUpdate> {
        self.step_counter += 1;
        let package = ui_state
            .get("package")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let elements = extract_elements(ui_state);
        if elements.is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "ui_state did not contain usable nodes or elements".to_string(),
            ));
        }

        let filtered = filter_functional_elements(elements);
        let fingerprint = page_fingerprint(&package, &filtered);
        let page_id = format!("page:{}", fingerprint);
        let summary = page_summary(&package, &filtered, task_hint);
        let candidate_actions = build_candidate_actions(&page_id, &filtered);
        let key_buttons = filtered
            .iter()
            .filter(|element| element.clickable && !element.dangerous)
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let input_fields = filtered
            .iter()
            .filter(|element| element.editable)
            .take(6)
            .cloned()
            .collect::<Vec<_>>();
        let navigation_entries = filtered
            .iter()
            .filter(|element| looks_like_navigation(element))
            .take(6)
            .cloned()
            .collect::<Vec<_>>();
        let dangerous_actions = filtered
            .iter()
            .filter(|element| element.dangerous)
            .take(6)
            .cloned()
            .collect::<Vec<_>>();

        let created = !self.pages.contains_key(&page_id);
        let replaced_stale_node = self
            .pages
            .get(&page_id)
            .map(|node| node.stale)
            .unwrap_or(false);
        let visits = self
            .pages
            .get(&page_id)
            .map(|node| node.visits + 1)
            .unwrap_or(1);

        self.pages.insert(
            page_id.clone(),
            PageNode {
                id: page_id.clone(),
                package,
                summary,
                fingerprint,
                key_buttons,
                input_fields,
                navigation_entries,
                dangerous_actions,
                visits,
                stale: false,
                last_seen_step: self.step_counter,
            },
        );
        self.current_page = Some(page_id.clone());

        Ok(MapUpdate {
            page_id,
            created,
            replaced_stale_node,
            candidate_actions,
        })
    }

    pub fn record_transition(
        &mut self,
        from_page: impl Into<String>,
        to_page: impl Into<String>,
        mut action: CandidateAction,
        success: bool,
    ) {
        let from_page = from_page.into();
        let to_page = to_page.into();
        action.from_page = from_page.clone();
        action.expected_target = Some(to_page.clone());

        if let Some(existing) = self.transitions.iter_mut().find(|transition| {
            transition.from_page == from_page
                && transition.to_page == to_page
                && transition.action.id == action.id
        }) {
            if success {
                existing.success_count += 1;
                existing.stale = false;
            } else {
                existing.failure_count += 1;
                existing.stale = existing.failure_count >= existing.success_count + 2;
            }
            existing.action = action;
            return;
        }

        self.transitions.push(PageTransition {
            from_page,
            to_page,
            action,
            success_count: usize::from(success),
            failure_count: usize::from(!success),
            stale: !success,
        });
    }

    pub fn mark_transition_failed(&mut self, from_page: &str, action_id: &str) {
        for transition in self.transitions.iter_mut().filter(|transition| {
            transition.from_page == from_page && transition.action.id == action_id
        }) {
            transition.failure_count += 1;
            transition.stale = transition.failure_count >= transition.success_count + 2;
        }
    }

    pub fn mark_page_stale(&mut self, page_id: &str) -> bool {
        let Some(page) = self.pages.get_mut(page_id) else {
            return false;
        };
        page.stale = true;
        true
    }

    pub fn local_view(
        &self,
        current_page: Option<&str>,
        hops: usize,
        task_query: Option<&str>,
    ) -> LocalMapView {
        let start = current_page
            .map(ToOwned::to_owned)
            .or_else(|| self.current_page.clone());
        let Some(start_id) = start else {
            return LocalMapView {
                current_page: None,
                nodes: Vec::new(),
                transitions: Vec::new(),
                candidate_actions: Vec::new(),
                omitted_node_count: self.pages.len(),
            };
        };

        let reachable = self.reachable_pages(&start_id, hops);
        let mut nodes = reachable
            .iter()
            .filter_map(|page_id| self.pages.get(page_id).cloned())
            .collect::<Vec<_>>();
        if let Some(query) = task_query {
            let hits = self.semantic_search(query, 4);
            for hit in hits {
                if !reachable.contains(&hit.page_id) {
                    if let Some(node) = self.pages.get(&hit.page_id) {
                        nodes.push(node.clone());
                    }
                }
            }
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        nodes.dedup_by(|left, right| left.id == right.id);

        let visible_ids = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        let transitions = self
            .transitions
            .iter()
            .filter(|transition| {
                !transition.stale
                    && visible_ids.contains(&transition.from_page)
                    && visible_ids.contains(&transition.to_page)
            })
            .cloned()
            .collect::<Vec<_>>();
        let candidate_actions = self
            .pages
            .get(&start_id)
            .map(|node| build_candidate_actions(&node.id, &page_elements_from_node(node)))
            .unwrap_or_default();
        let omitted_node_count = self.pages.len().saturating_sub(nodes.len());

        LocalMapView {
            current_page: Some(start_id),
            nodes,
            transitions,
            candidate_actions,
            omitted_node_count,
        }
    }

    pub fn semantic_search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let query_tokens = tokenize(query);
        let mut hits = self
            .pages
            .values()
            .filter_map(|page| {
                let haystack = format!(
                    "{} {} {} {}",
                    page.package,
                    page.summary,
                    page.key_buttons
                        .iter()
                        .map(element_label)
                        .collect::<Vec<_>>()
                        .join(" "),
                    page.input_fields
                        .iter()
                        .map(element_label)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                let page_tokens = tokenize(&haystack);
                let overlap = query_tokens
                    .iter()
                    .filter(|token| page_tokens.contains(*token))
                    .count();
                let substring_bonus = usize::from(
                    !query.trim().is_empty()
                        && haystack.to_lowercase().contains(&query.to_lowercase()),
                ) * 3;
                let matched = overlap * 2 + substring_bonus;
                let score = matched + usize::from(!page.stale);
                (matched > 0).then(|| SearchHit {
                    page_id: page.id.clone(),
                    score,
                    summary: page.summary.clone(),
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.page_id.cmp(&right.page_id))
        });
        hits.truncate(limit);
        hits
    }

    pub fn plan_path(&self, from_page: &str, target_page: &str) -> Option<Vec<PageTransition>> {
        if from_page == target_page {
            return Some(Vec::new());
        }

        let mut queue = VecDeque::from([from_page.to_string()]);
        let mut previous: BTreeMap<String, (String, PageTransition)> = BTreeMap::new();
        let mut seen = BTreeSet::from([from_page.to_string()]);

        while let Some(page_id) = queue.pop_front() {
            for transition in self
                .transitions
                .iter()
                .filter(|transition| transition.from_page == page_id && !transition.stale)
            {
                if !seen.insert(transition.to_page.clone()) {
                    continue;
                }
                previous.insert(
                    transition.to_page.clone(),
                    (page_id.clone(), transition.clone()),
                );
                if transition.to_page == target_page {
                    let mut cursor = target_page.to_string();
                    let mut path = Vec::new();
                    while let Some((prior, transition)) = previous.get(&cursor).cloned() {
                        path.push(transition);
                        cursor = prior;
                        if cursor == from_page {
                            break;
                        }
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(transition.to_page.clone());
            }
        }
        None
    }

    pub fn forget(&mut self, scope: ForgetScope) -> usize {
        match scope {
            ForgetScope::Page { page_id } => self.forget_pages([page_id].into_iter().collect()),
            ForgetScope::Query { query } => {
                let ids = self
                    .semantic_search(&query, usize::MAX)
                    .into_iter()
                    .map(|hit| hit.page_id)
                    .collect::<BTreeSet<_>>();
                self.forget_pages(ids)
            }
            ForgetScope::Stale => {
                let ids = self
                    .pages
                    .values()
                    .filter(|page| page.stale)
                    .map(|page| page.id.clone())
                    .collect::<BTreeSet<_>>();
                self.forget_pages(ids)
            }
        }
    }

    pub fn estimate_context_tokens(view: &LocalMapView) -> usize {
        let text_chars = view
            .nodes
            .iter()
            .map(|node| node.summary.len())
            .sum::<usize>()
            + view
                .transitions
                .iter()
                .map(|transition| {
                    transition.from_page.len()
                        + transition.to_page.len()
                        + transition.action.label.len()
                })
                .sum::<usize>()
            + view
                .candidate_actions
                .iter()
                .map(|action| action.label.len() + action.tool_name.len())
                .sum::<usize>();
        (text_chars / 4).max(1)
    }

    pub fn estimate_full_map_tokens(&self) -> usize {
        let text_chars = self
            .pages
            .values()
            .map(|node| {
                node.summary.len()
                    + node
                        .key_buttons
                        .iter()
                        .map(|element| element_label(element).len())
                        .sum::<usize>()
                    + node
                        .input_fields
                        .iter()
                        .map(|element| element_label(element).len())
                        .sum::<usize>()
                    + node
                        .dangerous_actions
                        .iter()
                        .map(|element| element_label(element).len())
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self
                .transitions
                .iter()
                .map(|transition| {
                    transition.from_page.len()
                        + transition.to_page.len()
                        + transition.action.label.len()
                })
                .sum::<usize>();
        (text_chars / 4).max(1)
    }

    fn reachable_pages(&self, start_id: &str, hops: usize) -> BTreeSet<String> {
        let mut seen = BTreeSet::from([start_id.to_string()]);
        let mut queue = VecDeque::from([(start_id.to_string(), 0usize)]);
        while let Some((page_id, depth)) = queue.pop_front() {
            if depth >= hops {
                continue;
            }
            for transition in self
                .transitions
                .iter()
                .filter(|transition| transition.from_page == page_id && !transition.stale)
            {
                if seen.insert(transition.to_page.clone()) {
                    queue.push_back((transition.to_page.clone(), depth + 1));
                }
            }
        }
        seen
    }

    fn forget_pages(&mut self, page_ids: BTreeSet<String>) -> usize {
        let count = page_ids.len();
        for page_id in &page_ids {
            self.pages.remove(page_id);
        }
        self.transitions.retain(|transition| {
            !page_ids.contains(&transition.from_page) && !page_ids.contains(&transition.to_page)
        });
        if self
            .current_page
            .as_ref()
            .map(|page_id| page_ids.contains(page_id))
            .unwrap_or(false)
        {
            self.current_page = None;
        }
        count
    }
}

pub fn sample_cross_app_map() -> AgentCoreResult<AppMapMemory> {
    let mut map = AppMapMemory::new();
    let home = map.observe_ui_state(
        &json!({
            "package": "com.example.shop",
            "nodes": [
                {"text": "Search", "class": "android.widget.EditText", "clickable": true, "editable": true, "bounds": "32,90,900,150"},
                {"text": "Orders", "class": "android.widget.Button", "clickable": true, "bounds": "32,170,300,230"},
                {"text": "Account", "class": "android.widget.Button", "clickable": true, "bounds": "780,170,1040,230"}
            ]
        }),
        Some("buy a charger"),
    )?;
    let orders = map.observe_ui_state(
        &json!({
            "package": "com.example.shop",
            "nodes": [
                {"text": "Order #2048", "class": "android.widget.TextView", "clickable": true, "bounds": "40,220,800,280"},
                {"text": "Back", "class": "android.widget.Button", "clickable": true, "bounds": "30,60,150,120"}
            ]
        }),
        Some("order list"),
    )?;
    let detail = map.observe_ui_state(
        &json!({
            "package": "com.example.shop",
            "nodes": [
                {"text": "Order Details", "class": "android.widget.TextView", "clickable": false},
                {"text": "Share", "class": "android.widget.Button", "clickable": true},
                {"text": "Cancel Order", "class": "android.widget.Button", "clickable": true}
            ]
        }),
        Some("order details"),
    )?;
    let notes = map.observe_ui_state(
        &json!({
            "package": "com.example.notes",
            "nodes": [
                {"text": "New note", "class": "android.widget.Button", "clickable": true},
                {"text": "Search notes", "class": "android.widget.EditText", "clickable": true, "editable": true}
            ]
        }),
        Some("save order summary"),
    )?;

    let home_id = home.page_id;
    let orders_id = orders.page_id;
    let detail_id = detail.page_id;
    let notes_id = notes.page_id;
    map.record_transition(
        home_id.clone(),
        orders_id.clone(),
        CandidateAction {
            id: "click:orders".to_string(),
            label: "Orders".to_string(),
            kind: CandidateActionKind::ClickText,
            tool_name: "click_at".to_string(),
            arguments: json!({"x": 166, "y": 200}),
            from_page: home_id.clone(),
            expected_target: Some(orders_id.clone()),
            read_only: false,
            dangerous: false,
        },
        true,
    );
    map.record_transition(
        orders_id.clone(),
        detail_id.clone(),
        CandidateAction {
            id: "click:order-2048".to_string(),
            label: "Order #2048".to_string(),
            kind: CandidateActionKind::ClickText,
            tool_name: "click_at".to_string(),
            arguments: json!({"x": 420, "y": 250}),
            from_page: orders_id.clone(),
            expected_target: Some(detail_id.clone()),
            read_only: false,
            dangerous: false,
        },
        true,
    );
    map.record_transition(
        detail_id.clone(),
        notes_id.clone(),
        CandidateAction {
            id: "open-app:notes".to_string(),
            label: "Open Notes".to_string(),
            kind: CandidateActionKind::OpenApp,
            tool_name: "open_app".to_string(),
            arguments: json!({"text": "Notes"}),
            from_page: detail_id,
            expected_target: Some(notes_id),
            read_only: false,
            dangerous: false,
        },
        true,
    );
    map.current_page = Some(home_id);
    Ok(map)
}

fn extract_elements(ui_state: &Value) -> Vec<UiElementSummary> {
    let array = ui_state
        .get("nodes")
        .or_else(|| ui_state.get("elements"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    array
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let description = value
                .get("description")
                .or_else(|| value.get("contentDescription"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let class_name = value
                .get("class")
                .or_else(|| value.get("class_name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let bounds = value
                .get("bounds")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let label = if text.is_empty() {
                description.clone()
            } else {
                text.clone()
            };
            UiElementSummary {
                stable_id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "node:{}:{}",
                            index,
                            sanitize_token(&format!("{label}{class_name}"))
                        )
                    }),
                text,
                description,
                class_name,
                clickable: value
                    .get("clickable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                editable: value
                    .get("editable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                dangerous: is_dangerous_label(&label),
                bounds,
            }
        })
        .collect()
}

fn filter_functional_elements(elements: Vec<UiElementSummary>) -> Vec<UiElementSummary> {
    let mut seen = BTreeSet::new();
    elements
        .into_iter()
        .filter(|element| {
            element.clickable
                || element.editable
                || element.dangerous
                || looks_like_navigation(element)
                || !element_label(element).is_empty()
        })
        .filter(|element| seen.insert(element_signature(element)))
        .take(40)
        .collect()
}

fn build_candidate_actions(page_id: &str, elements: &[UiElementSummary]) -> Vec<CandidateAction> {
    let mut actions = Vec::new();
    for element in elements {
        let label = element_label(element);
        if label.is_empty() || element.dangerous {
            continue;
        }
        if element.clickable {
            let (tool_name, arguments) =
                if let Some((x, y)) = bounds_center(element.bounds.as_deref()) {
                    ("click_at".to_string(), json!({"x": x, "y": y}))
                } else {
                    (
                        "click".to_string(),
                        json!({"index": element_index(element)}),
                    )
                };
            actions.push(CandidateAction {
                id: format!("click:{}", sanitize_token(&label)),
                label: label.clone(),
                kind: CandidateActionKind::ClickText,
                tool_name,
                arguments,
                from_page: page_id.to_string(),
                expected_target: None,
                read_only: false,
                dangerous: false,
            });
        }
        if element.editable {
            actions.push(CandidateAction {
                id: format!("type:{}", sanitize_token(&label)),
                label,
                kind: CandidateActionKind::TypeText,
                tool_name: "type".to_string(),
                arguments: json!({"text": "<task_text>", "index": element_index(element)}),
                from_page: page_id.to_string(),
                expected_target: None,
                read_only: false,
                dangerous: false,
            });
        }
    }
    actions.push(CandidateAction {
        id: "system:back".to_string(),
        label: "Back".to_string(),
        kind: CandidateActionKind::SystemBack,
        tool_name: "system_button".to_string(),
        arguments: json!({"button": "back"}),
        from_page: page_id.to_string(),
        expected_target: None,
        read_only: false,
        dangerous: false,
    });
    actions
}

fn page_elements_from_node(node: &PageNode) -> Vec<UiElementSummary> {
    let mut elements = Vec::new();
    elements.extend(node.key_buttons.clone());
    elements.extend(node.input_fields.clone());
    elements.extend(node.navigation_entries.clone());
    elements.extend(node.dangerous_actions.clone());
    elements
}

fn page_fingerprint(package: &str, elements: &[UiElementSummary]) -> String {
    let mut labels = elements
        .iter()
        .map(element_label)
        .filter(|label| !label.is_empty())
        .map(|label| sanitize_token(&label))
        .take(12)
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    sanitize_token(&format!("{package}:{}", labels.join("|")))
}

fn page_summary(package: &str, elements: &[UiElementSummary], task_hint: Option<&str>) -> String {
    let buttons = elements
        .iter()
        .filter(|element| element.clickable && !element.dangerous)
        .map(element_label)
        .filter(|label| !label.is_empty())
        .take(6)
        .collect::<Vec<_>>();
    let inputs = elements
        .iter()
        .filter(|element| element.editable)
        .map(element_label)
        .filter(|label| !label.is_empty())
        .take(4)
        .collect::<Vec<_>>();
    let dangerous = elements
        .iter()
        .filter(|element| element.dangerous)
        .map(element_label)
        .filter(|label| !label.is_empty())
        .take(4)
        .collect::<Vec<_>>();
    format!(
        "package={package}; task_hint={}; buttons=[{}]; inputs=[{}]; dangerous=[{}]",
        task_hint.unwrap_or(""),
        buttons.join(", "),
        inputs.join(", "),
        dangerous.join(", ")
    )
}

fn looks_like_navigation(element: &UiElementSummary) -> bool {
    let label = element_label(element).to_lowercase();
    matches!(
        label.as_str(),
        "back" | "home" | "orders" | "account" | "settings" | "search" | "new note"
    ) || element.class_name.to_lowercase().contains("tab")
}

fn is_dangerous_label(label: &str) -> bool {
    let lowered = label.to_lowercase();
    [
        "delete", "cancel", "remove", "logout", "sign out", "pay", "purchase",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn element_label(element: &UiElementSummary) -> String {
    if !element.text.trim().is_empty() {
        element.text.trim().to_string()
    } else {
        element.description.trim().to_string()
    }
}

fn element_signature(element: &UiElementSummary) -> String {
    format!(
        "{}|{}|{}|{}",
        element_label(element).to_lowercase(),
        element.class_name,
        element.clickable,
        element.editable
    )
}

fn element_index(element: &UiElementSummary) -> usize {
    element
        .stable_id
        .split(':')
        .nth(1)
        .and_then(|index| index.parse::<usize>().ok())
        .unwrap_or(0)
}

fn bounds_center(bounds: Option<&str>) -> Option<(i64, i64)> {
    let bounds = bounds?;
    let numbers = bounds
        .split(|character: char| !character.is_ascii_digit() && character != '-')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<i64>().ok())
        .collect::<Vec<_>>();
    if numbers.len() != 4 {
        return None;
    }
    Some(((numbers[0] + numbers[2]) / 2, (numbers[1] + numbers[3]) / 2))
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_lowercase())
        .collect()
}

fn sanitize_token(text: &str) -> String {
    let mut token = text
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while token.contains("--") {
        token = token.replace("--", "-");
    }
    token.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_page_nodes_and_candidates_from_ui_state() {
        let mut map = AppMapMemory::new();
        let update = map
            .observe_ui_state(
                &json!({
                    "package": "com.example.settings",
                    "nodes": [
                        {"text": "Account security", "class": "android.widget.Button", "clickable": true},
                        {"text": "Password", "class": "android.widget.EditText", "editable": true},
                        {"text": "Delete account", "class": "android.widget.Button", "clickable": true}
                    ]
                }),
                Some("go to account security"),
            )
            .unwrap();

        assert!(update.created);
        assert_eq!(map.page_count(), 1);
        assert!(update
            .candidate_actions
            .iter()
            .any(|action| action.label == "Account security"));
        let page = map.pages().next().unwrap();
        assert_eq!(page.dangerous_actions[0].text, "Delete account");
    }

    #[test]
    fn searches_places_and_returns_local_view_without_full_map() {
        let map = sample_cross_app_map().unwrap();
        let home = map.current_page().unwrap().to_string();
        let hit = map.semantic_search("Share", 1).remove(0);
        let path = map.plan_path(&home, &hit.page_id).unwrap();
        let view = map.local_view(Some(&home), 1, Some("Share"));

        assert_eq!(path.len(), 2);
        assert!(view.nodes.len() < map.page_count());
        assert!(AppMapMemory::estimate_context_tokens(&view) < map.estimate_full_map_tokens());
    }

    #[test]
    fn forget_removes_pages_and_dependent_paths() {
        let mut map = sample_cross_app_map().unwrap();
        let removed = map.forget(ForgetScope::Query {
            query: "notes".to_string(),
        });

        assert_eq!(removed, 1);
        assert_eq!(map.transition_count(), 2);
        assert!(map.semantic_search("notes", 1).is_empty());
    }

    #[test]
    fn failed_paths_become_stale_after_repeated_misses() {
        let mut map = sample_cross_app_map().unwrap();
        let transition = map.transitions()[0].clone();
        map.mark_transition_failed(&transition.from_page, &transition.action.id);
        map.mark_transition_failed(&transition.from_page, &transition.action.id);
        map.mark_transition_failed(&transition.from_page, &transition.action.id);

        assert!(map.transitions()[0].stale);
    }

    #[test]
    fn stale_page_node_is_refreshed_by_next_matching_observation() {
        let mut map = AppMapMemory::new();
        let ui_state = json!({
            "package": "com.example.settings",
            "nodes": [
                {"text": "Account security", "class": "android.widget.Button", "clickable": true},
                {"text": "Password", "class": "android.widget.EditText", "editable": true}
            ]
        });
        let first = map
            .observe_ui_state(&ui_state, Some("initial account security view"))
            .unwrap();

        assert!(map.mark_page_stale(&first.page_id));
        assert!(
            map.pages()
                .find(|page| page.id == first.page_id)
                .unwrap()
                .stale
        );

        let refreshed = map
            .observe_ui_state(&ui_state, Some("refreshed account security view"))
            .unwrap();

        assert_eq!(refreshed.page_id, first.page_id);
        assert!(!refreshed.created);
        assert!(refreshed.replaced_stale_node);
        assert!(
            !map.pages()
                .find(|page| page.id == first.page_id)
                .unwrap()
                .stale
        );
    }
}
