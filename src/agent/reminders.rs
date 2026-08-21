//! ReminderQueue — Multi-turn persistent reminder queue
//!
//! Replaces the old `pending_reminder: Option<String>` single-turn lifecycle design.
//!
//! Old design flaws:
//! - `pending_reminder` is set to `= None` immediately after injection in build_request()
//! - LLM responds with text instead of tool calls → reminder context permanently lost
//! - force_output strong wording only lasts one turn
//!
//! New design:
//! - Specify max_deliveries when enqueuing (persists for N turns)
//! - format_for_prompt() does not delete after injection, instead increments delivered_count
//! - After LLM actually calls the relevant tool, remove via clear_by_prefix()
//! - Priorities Critical/High/Normal come with visual markers

/// Reminder priority
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderPriority {
    /// 🚫 Highest priority — force_output and other forced output reminders
    Critical,
    /// ⚠️ High priority — error avoidance, parameter correction
    High,
    /// ▶ Normal priority — general guidance
    Normal,
}

/// Single reminder
#[derive(Debug, Clone)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    pub max_deliveries: u32,
    pub delivered_count: u32,
    pub priority: ReminderPriority,
    pub category: ReminderCategory,
}

/// Reminder category — determines injection format
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderCategory {
    /// 🛡 Security intercept — already delivered via tool output, kept as label
    SecurityIntercept,
    /// 🔧 Deviation correction — path expansion, loop detection, etc.
    DeviationCorrect,
    /// 📋 Flow reminder — planner memory_update, etc.
    FlowReminder,
}

impl Reminder {
    /// Format as user message — only for DeviationCorrect and FlowReminder.
    ///
    /// SecurityIntercept reminders are delivered via tool output and should
    /// NOT be formatted as user messages; this method returns None for them.
    pub fn format_as_user_message(&self) -> Option<String> {
        let label = match self.category {
            ReminderCategory::SecurityIntercept => return None,
            ReminderCategory::DeviationCorrect => "偏差纠正",
            ReminderCategory::FlowReminder => "流程提醒",
        };
        Some(format!(
            "## Reminder: {}\n事实: {}\n? 是否已足够？如需继续，忽略此提示。",
            label, self.text
        ))
    }
}

/// Persistent reminder queue
///
/// Reminders persist in the queue across multiple turns, not consumed by a single injection.
/// Automatically removed when delivered_count >= max_deliveries.
/// Preemptively cleared via clear_by_prefix() when LLM calls the relevant tool.
pub struct ReminderQueue {
    items: Vec<Reminder>,
}

impl Default for ReminderQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ReminderQueue {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Enqueue a reminder
    ///
    /// - `text`: reminder text
    /// - `max_deliveries`: maximum number of injection turns (default 5)
    /// - `priority`: priority
    /// - `category`: reminder category
    pub fn enqueue(
        &mut self,
        text: String,
        max_deliveries: u32,
        priority: ReminderPriority,
        category: ReminderCategory,
    ) {
        self.items.push(Reminder {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            max_deliveries,
            delivered_count: 0,
            priority,
            category,
        });
    }

    /// Generate current turn's reminder text, suitable for injection into system prompt.
    ///
    /// Each reminder's delivered_count is incremented by 1.
    /// Automatically removed when exceeding max_deliveries.
    /// Returns None if queue is empty.
    pub fn format_for_prompt(&mut self) -> Option<String> {
        // Remove reminders that have been delivered enough times
        self.items.retain(|r| r.delivered_count < r.max_deliveries);

        if self.items.is_empty() {
            return None;
        }

        let mut parts: Vec<String> = Vec::new();
        parts.push("### ACTIVE REMINDERS (persist until resolved)\n".to_string());

        // Sort by priority: Critical first
        self.items.sort_by_key(|r| match r.priority {
            ReminderPriority::Critical => 0u8,
            ReminderPriority::High => 1,
            ReminderPriority::Normal => 2,
        });

        for r in &mut self.items {
            r.delivered_count += 1;
            let marker = match r.priority {
                ReminderPriority::Critical => "\u{1F6AB}", // 🚫
                ReminderPriority::High => "[高优先级]",    // ⚠️
                ReminderPriority::Normal => "\u{25B6}",    // ▶
            };
            parts.push(format!("{} [REMINDER] {}", marker, r.text));
        }

        Some(parts.join("\n"))
    }

    /// When LLM calls a tool matching prefix, clear related reminders.
    ///
    /// Matches tool name prefix and word-order variants like "write_file".
    /// Uses keyword set extraction for fuzzy matching to avoid cleanup failure when abbreviated names are used at enqueue time vs full names at cleanup time.
    pub fn clear_by_prefix(&mut self, prefix: &str) {
        // Extract keywords from prefix (split by :: and _)
        let prefix_words: Vec<String> = prefix
            .split([':', '_'])
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect();

        self.items.retain(|r| {
            let text_lower = r.text.to_lowercase();
            // Strategy 1: raw string contains
            if text_lower.contains(&prefix.to_lowercase()) {
                return false;
            }
            // Strategy 2: normalized form (:: → _) contains
            let norm_prefix = prefix.to_lowercase();
            if text_lower.contains(&norm_prefix) {
                return false;
            }
            // Strategy 3: keyword set match (all keywords appear in text)
            if !prefix_words.is_empty() && prefix_words.iter().all(|w| text_lower.contains(w)) {
                return false;
            }
            true
        });
    }

    /// Clear all reminders (when task completes or is cancelled)
    pub fn clear_all(&mut self) {
        self.items.clear();
    }

    /// Get current active reminder count
    pub fn active_count(&self) -> usize {
        self.items.len()
    }

    /// For event emission: get snapshot of all current reminders
    pub fn snapshot(&self) -> Vec<(String, u32, u32, String)> {
        self.items
            .iter()
            .map(|r| {
                let kind = match r.priority {
                    ReminderPriority::Critical => "critical",
                    ReminderPriority::High => "high",
                    ReminderPriority::Normal => "normal",
                };
                (
                    kind.to_string(),
                    r.delivered_count,
                    r.max_deliveries,
                    r.text.clone(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_and_format() {
        let mut q = ReminderQueue::new();
        assert!(q.format_for_prompt().is_none());

        q.enqueue(
            "请调用 write_file".to_string(),
            3,
            ReminderPriority::Critical,
            ReminderCategory::FlowReminder,
        );
        assert_eq!(q.active_count(), 1);

        let out = q.format_for_prompt().unwrap();
        assert!(out.contains("REMINDER"), "first delivery: {}", out);
        assert!(out.contains("write_file"), "content present: {}", out);
        assert!(out.contains("🚫"), "critical marker: {}", out);
        assert_eq!(q.active_count(), 1, "still active after 1/3");
    }

    #[test]
    fn test_multi_delivery() {
        let mut q = ReminderQueue::new();
        q.enqueue(
            "test".to_string(),
            2,
            ReminderPriority::Normal,
            ReminderCategory::FlowReminder,
        );

        assert!(q.format_for_prompt().is_some()); // delivery 1
        assert!(q.format_for_prompt().is_some()); // delivery 2
        assert!(q.format_for_prompt().is_none()); // expired
    }

    #[test]
    fn test_clear_by_prefix() {
        let mut q = ReminderQueue::new();
        q.enqueue(
            "write_file 提醒".to_string(),
            5,
            ReminderPriority::Critical,
            ReminderCategory::FlowReminder,
        );
        q.enqueue(
            "execute_shell 提醒".to_string(),
            5,
            ReminderPriority::High,
            ReminderCategory::DeviationCorrect,
        );
        assert_eq!(q.active_count(), 2);

        // Clear using write_file form, should match write_file reminder
        q.clear_by_prefix("write_file");
        assert_eq!(q.active_count(), 1);
        assert!(q.items[0].text.contains("shell"));
    }

    #[test]
    fn test_clear_by_prefix_alt_form() {
        let mut q = ReminderQueue::new();
        q.enqueue(
            "file::read 提醒".to_string(),
            5,
            ReminderPriority::Critical,
            ReminderCategory::FlowReminder,
        );
        q.enqueue(
            "read_file 提醒".to_string(),
            5,
            ReminderPriority::High,
            ReminderCategory::FlowReminder,
        );
        assert_eq!(q.active_count(), 2);

        // Clear using read_file form, should match both file::read and read_file
        q.clear_by_prefix("read_file");
        assert_eq!(q.active_count(), 0);
    }

    #[test]
    fn test_priority_order() {
        let mut q = ReminderQueue::new();
        q.enqueue(
            "normal".to_string(),
            5,
            ReminderPriority::Normal,
            ReminderCategory::FlowReminder,
        );
        q.enqueue(
            "critical".to_string(),
            5,
            ReminderPriority::Critical,
            ReminderCategory::DeviationCorrect,
        );
        q.enqueue(
            "high".to_string(),
            5,
            ReminderPriority::High,
            ReminderCategory::FlowReminder,
        );

        let out = q.format_for_prompt().unwrap();
        let crit_pos = out.find("critical").unwrap();
        let high_pos = out.find("high").unwrap();
        let norm_pos = out.find("normal").unwrap();
        assert!(crit_pos < high_pos, "critical before high");
        assert!(high_pos < norm_pos, "high before normal");
    }

    #[test]
    fn test_clear_all() {
        let mut q = ReminderQueue::new();
        q.enqueue(
            "a".to_string(),
            5,
            ReminderPriority::Normal,
            ReminderCategory::FlowReminder,
        );
        q.enqueue(
            "b".to_string(),
            5,
            ReminderPriority::Normal,
            ReminderCategory::FlowReminder,
        );
        q.clear_all();
        assert!(q.is_empty());
    }
}
