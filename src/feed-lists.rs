//! Settings > Lists: the MDBList rows on Home, added from your own lists, MDBList's most
//! liked or a search, removed, and moved up and down. They are kept in preferences; `main`
//! puts them in place of feeds.json's `mdblist:` rows and restarts the feed worker.

use crate::feeds::Outgoing;
use crate::model::FeedList;
use crate::state::{Control, Direction, Effect, Page, State, Subpage, Target, TextField};

impl State {
    /// The lists on Home, in order: as Settings left them, else feeds.json's.
    pub fn lists(&self) -> Vec<FeedList> {
        self.preferences
            .feed_lists
            .clone()
            .unwrap_or_else(|| self.feeds.config_lists.clone())
    }

    fn set_lists(&mut self, lists: Vec<FeedList>) -> Vec<Effect> {
        self.preferences.feed_lists = Some(lists);
        self.feeds.lists_changed = true;
        self.dirty = true;
        vec![Effect::Save]
    }

    /// Opens Add a list, asking MDBList for your lists and its most liked ones.
    fn open_add_list(&mut self) {
        self.subpage = Some(Subpage::AddList);
        self.focus = Target::ListSearch;
        self.scroll = 0.;
        self.feeds.list_query.clear();
        self.feeds.list_results = None;
        self.feeds.outbox.push(Outgoing::Lists(None));
        if !self.preferences.reduced_motion {
            self.panel_started = Some(std::time::Instant::now());
        }
        self.dirty = true;
    }

    /// The keyboard closed on a search: ask MDBList.
    pub fn search_lists(&mut self) -> Vec<Effect> {
        let query = self.feeds.list_query.trim().to_owned();
        self.feeds.list_results = None;
        self.feeds
            .outbox
            .push(Outgoing::Lists(Some(query).filter(|q| !q.is_empty())));
        self.dirty = true;
        vec![]
    }

    /// MDBList's answer: lists to add.
    pub fn lists_found(&mut self, result: Result<Vec<crate::feeds::ListInfo>, String>) {
        if self.subpage != Some(Subpage::AddList) {
            return;
        }
        // Ones already on Home are left out.
        let have: Vec<String> = self.lists().into_iter().map(|list| list.source).collect();
        self.feeds.list_results = Some(result.map(|lists| {
            lists
                .into_iter()
                .filter(|list| !have.contains(&list.source))
                .collect()
        }));
        if matches!(self.focus, Target::ListResult(_)) {
            self.focus = Target::ListSearch;
        }
        self.dirty = true;
    }

    fn found_count(&self) -> usize {
        self.feeds
            .list_results
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map_or(0, Vec::len)
    }

    /// OK on Settings > Lists. None when focus is elsewhere.
    pub fn lists_activate(&mut self) -> Option<Vec<Effect>> {
        if self.page != Page::Settings {
            return None;
        }
        Some(match self.focus.clone() {
            Target::ListAdd => {
                self.feeds.list_moving = None;
                self.open_add_list();
                vec![]
            }
            // OK picks a list up to move; OK again puts it down.
            Target::ListItem(index) => {
                self.feeds.list_moving = match self.feeds.list_moving {
                    Some(_) => None,
                    None => Some(index),
                };
                self.dirty = true;
                vec![]
            }
            Target::ListRemove(index) => {
                let mut lists = self.lists();
                if index >= lists.len() {
                    return Some(vec![]);
                }
                let removed = lists.remove(index);
                self.notify(format!("Removed {}", removed.title));
                self.focus = if lists.is_empty() {
                    Target::ListAdd
                } else {
                    Target::ListRemove(index.min(lists.len() - 1))
                };
                self.set_lists(lists)
            }
            Target::ListSearch => {
                self.open_keyboard(TextField::ListSearch);
                vec![]
            }
            Target::ListResult(index) => {
                let Some(found) = self
                    .feeds
                    .list_results
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .and_then(|lists| lists.get(index))
                    .cloned()
                else {
                    return Some(vec![]);
                };
                let mut lists = self.lists();
                lists.push(FeedList {
                    title: found.title.clone(),
                    source: found.source.clone(),
                });
                self.notify(format!("Added {} to Home", found.title));
                if let Some(Ok(results)) = &mut self.feeds.list_results {
                    results.remove(index);
                }
                let left = self.found_count();
                self.focus = if left == 0 {
                    Target::ListSearch
                } else {
                    Target::ListResult(index.min(left - 1))
                };
                self.set_lists(lists)
            }
            _ => return None,
        })
    }

    /// Remote navigation on Settings > Lists and Add a list. None when focus is elsewhere.
    pub fn lists_direction(
        &mut self,
        direction: Direction,
        controls: &[Control],
    ) -> Option<Vec<Effect>> {
        if self.page != Page::Settings {
            return None;
        }
        let count = self.lists().len();
        match self.focus.clone() {
            // Moving a list: Up and Down carry it.
            Target::ListItem(index) if self.feeds.list_moving.is_some() => {
                let next = match direction {
                    Direction::Up => index.checked_sub(1),
                    Direction::Down => (index + 1 < count).then_some(index + 1),
                    _ => None,
                };
                let Some(next) = next else {
                    return Some(vec![]);
                };
                let mut lists = self.lists();
                lists.swap(index, next);
                self.feeds.list_moving = Some(next);
                self.focus(Target::ListItem(next), controls);
                Some(self.set_lists(lists))
            }
            Target::ListAdd => {
                match direction {
                    Direction::Up => self.focus(Target::Nav(2), controls),
                    Direction::Down if count > 0 => self.focus(Target::ListItem(0), controls),
                    Direction::Left => self.focus(Target::Sidebar(self.section), controls),
                    Direction::Right => return Some(self.activate()),
                    _ => {}
                }
                Some(vec![])
            }
            target @ (Target::ListItem(_) | Target::ListRemove(_)) => {
                let (index, remove) = match target {
                    Target::ListItem(index) => (index, false),
                    Target::ListRemove(index) => (index, true),
                    _ => unreachable!(),
                };
                let at = |index: usize| {
                    if remove {
                        Target::ListRemove(index)
                    } else {
                        Target::ListItem(index)
                    }
                };
                let next = match direction {
                    Direction::Up if index == 0 => Target::ListAdd,
                    Direction::Up => at(index - 1),
                    Direction::Down if index + 1 < count => at(index + 1),
                    Direction::Down => return Some(vec![]),
                    Direction::Right if !remove => Target::ListRemove(index),
                    Direction::Right => return Some(vec![]),
                    Direction::Left if remove => Target::ListItem(index),
                    Direction::Left => Target::Sidebar(self.section),
                };
                self.focus(next, controls);
                Some(vec![])
            }
            Target::ListSearch | Target::ListResult(_) => {
                let found = self.found_count();
                let index = match self.focus {
                    Target::ListResult(index) => Some(index),
                    _ => None,
                };
                match (direction, index) {
                    (Direction::Left, _) => self.close_subpage(),
                    (Direction::Up, None) => self.focus(Target::Nav(2), controls),
                    (Direction::Up, Some(0)) => self.focus(Target::ListSearch, controls),
                    (Direction::Up, Some(index)) => {
                        self.focus(Target::ListResult(index - 1), controls);
                    }
                    (Direction::Down, None) if found > 0 => {
                        self.focus(Target::ListResult(0), controls);
                    }
                    (Direction::Down, Some(index)) if index + 1 < found => {
                        self.focus(Target::ListResult(index + 1), controls);
                    }
                    _ => {}
                }
                Some(vec![])
            }
            _ => None,
        }
    }
}
