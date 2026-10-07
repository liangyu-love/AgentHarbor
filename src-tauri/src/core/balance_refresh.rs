use std::collections::{HashMap, HashSet};

pub const MAX_INTERVAL_MINUTES: u32 = 10_080;

pub struct Target {
    pub id: String,
    pub interval_minutes: u32,
    pub checked_at: i64,
}

/// Attempts are remembered even when a query fails. A failed endpoint is retried
/// at its configured cadence, rather than on every scheduler tick.
#[derive(Default)]
pub struct Schedule {
    attempted_at: HashMap<String, i64>,
}

impl Schedule {
    pub fn due(&mut self, targets: &[Target], now: i64) -> Vec<String> {
        let active: HashSet<_> = targets
            .iter()
            .filter(|target| target.interval_minutes > 0)
            .map(|target| target.id.as_str())
            .collect();
        self.attempted_at
            .retain(|id, _| active.contains(id.as_str()));
        let mut due = Vec::new();
        for target in targets {
            if target.interval_minutes == 0 {
                continue;
            }
            let previous = self
                .attempted_at
                .get(&target.id)
                .copied()
                .unwrap_or(target.checked_at)
                .max(target.checked_at);
            if now.saturating_sub(previous) >= i64::from(target.interval_minutes) * 60 {
                self.attempted_at.insert(target.id.clone(), now);
                due.push(target.id.clone());
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, minutes: u32, checked_at: i64) -> Target {
        Target {
            id: id.into(),
            interval_minutes: minutes,
            checked_at,
        }
    }

    #[test]
    fn respects_zero_minutes_due_time_and_failures() {
        let mut schedule = Schedule::default();
        let targets = [
            target("off", 0, 0),
            target("one", 1, 100),
            target("five", 5, 100),
        ];
        assert!(schedule.due(&targets, 159).is_empty());
        assert_eq!(schedule.due(&targets, 160), ["one"]);
        // No successful balance is required to pace retries.
        assert!(schedule.due(&targets, 175).is_empty());
        assert_eq!(schedule.due(&targets, 400), ["one", "five"]);
    }

    #[test]
    fn observes_manual_queries_interval_changes_and_removal() {
        let mut schedule = Schedule::default();
        assert_eq!(schedule.due(&[target("one", 1, 0)], 100), ["one"]);
        assert!(schedule.due(&[target("one", 1, 150)], 200).is_empty());
        assert_eq!(schedule.due(&[target("one", 1, 150)], 210), ["one"]);
        assert!(schedule.due(&[target("one", 5, 150)], 300).is_empty());
        assert!(schedule.due(&[], 310).is_empty());
        assert!(schedule.attempted_at.is_empty());
    }
}
