//! Cold imports keep eight full pages per cache file. A page number identifies
//! candidates only. The current index determines which analyses can be reused.
use super::{Analysis, Client, Page};
use crate::library::cache::{REFRESH_INTERVAL, Ticket};
use anyhow::Result;
use std::collections::HashSet;

const PAGES_PER_FILE: usize = 8;

pub(super) struct ColdPages<'a> {
    ticket: &'a Ticket,
    width: usize,
    pages: Vec<Vec<Analysis>>,
    dirty: bool,
}

impl<'a> ColdPages<'a> {
    pub(super) fn new(ticket: &'a Ticket) -> Self {
        Self {
            ticket,
            width: 0,
            pages: Vec::new(),
            dirty: false,
        }
    }

    pub(super) async fn fetch(
        &mut self,
        api: &Client,
        page: usize,
        allowed: &HashSet<String>,
    ) -> Result<Page<Analysis>> {
        let offset = (page - 1) % PAGES_PER_FILE;
        let part = self.ticket.checkpoint((page - 1) / PAGES_PER_FILE);
        if offset == 0 {
            let read = part.clone();
            self.pages = match tokio::task::spawn_blocking(move || {
                read.read::<Vec<Vec<Analysis>>>(REFRESH_INTERVAL)
            })
            .await?
            {
                Ok(Some(hit)) if !hit.stale && hit.value.len() == PAGES_PER_FILE => hit.value,
                Ok(_) => Vec::new(),
                Err(error) => {
                    tracing::warn!(%error, "Read AudioMuse checkpoint");
                    Vec::new()
                }
            };
            self.pages.resize_with(PAGES_PER_FILE, Vec::new);
            self.dirty = false;
        }
        let saved = &self.pages[offset];
        if page > 1 && !saved.is_empty() {
            let mut index = api.sync_page(page, self.width, true).await?;
            // Index and metadata endpoints may cap page sizes differently.
            // Only complete, aligned pages qualify. Fetch the tail in full.
            if index.has_more
                && index.tracks.len() == self.width
                && index
                    .tracks
                    .iter()
                    .filter(|t| allowed.contains(&t.id))
                    .map(|t| (&t.id, &t.fp))
                    .eq(saved.iter().map(|t| (&t.id, &t.fp)))
            {
                for (track, cached) in index
                    .tracks
                    .iter_mut()
                    .filter(|t| allowed.contains(&t.id))
                    .zip(saved)
                {
                    track.clone_from(cached);
                }
                return Ok(index);
            }
        }
        let full = api.sync_page(page, 1000, false).await?;
        if page == 1 {
            self.width = full.tracks.len();
        }
        self.pages[offset] = full
            .tracks
            .iter()
            .filter(|t| allowed.contains(&t.id))
            .cloned()
            .collect();
        self.dirty = true;
        Ok(full)
    }

    pub(super) async fn save_group(&mut self, page: usize) -> Result<()> {
        if page % PAGES_PER_FILE == 0 && self.dirty {
            let part = self.ticket.checkpoint((page - 1) / PAGES_PER_FILE);
            let pages = std::mem::take(&mut self.pages);
            tokio::task::spawn_blocking(move || part.write(&pages)).await??;
        }
        Ok(())
    }
}
