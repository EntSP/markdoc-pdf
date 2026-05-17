//! Greedy bin-packing of `Block`s into pages, with text-block splitting
//! at line boundaries when a paragraph doesn't fit on the current page.
//!
//! Algorithm:
//!   - Maintain a current page accumulator and a worklist of remaining
//!     blocks.
//!   - For each block: if it fits, push it. Otherwise call
//!     `Block::try_split(remaining)`:
//!       * `Whole`  → fits unchanged (accumulator was wrong; place it).
//!       * `Split(head, tail)` → push head, flush page, prepend tail
//!         to the worklist.
//!       * `NoFit`  → block can't be split or zero lines fit. Flush
//!         current page (if any) and place block on a fresh page.
//!         If even on a fresh page it doesn't fit, place anyway —
//!         overflow is tolerated.

use std::collections::VecDeque;

use super::block::{Block, SplitOutcome};

/// Footnote-aware pagination. The `footnote_pool` callback gets the
/// list of footnote numbers attached to the blocks currently parked on
/// the candidate page; it returns the height that the resulting pool
/// would consume *and* the actual pool blocks, in case the caller
/// later wants to render them. We re-evaluate the pool on every block
/// addition so growing pool height steals from the body budget; if a
/// block plus its footnote bodies no longer fit, it gets bumped to the
/// next page (or split, for splittable text blocks).
///
/// `pool_for` returns `(height, pool_blocks)` for the supplied
/// footnote numbers, or `(0.0, Vec::new())` when there are none.
pub fn paginate_with_footnotes(
    blocks: Vec<Block>,
    page_budget: f32,
    mut pool_for: impl FnMut(&[u32]) -> (f32, Vec<Block>),
) -> Vec<(Vec<Block>, Vec<Block>)> {
    let mut pages: Vec<(Vec<Block>, Vec<Block>)> = Vec::new();
    let mut current: Vec<Block> = Vec::new();
    let mut current_body_height = 0.0_f32;
    let mut current_pool_height = 0.0_f32;
    let mut current_pool_blocks: Vec<Block> = Vec::new();
    let mut current_numbers: Vec<u32> = Vec::new();

    let mut work: VecDeque<Block> = blocks.into();

    let flush = |pages: &mut Vec<(Vec<Block>, Vec<Block>)>,
                 current: &mut Vec<Block>,
                 pool: &mut Vec<Block>,
                 body_height: &mut f32,
                 pool_height: &mut f32,
                 numbers: &mut Vec<u32>| {
        let body = std::mem::take(current);
        let pool_blocks = std::mem::take(pool);
        pages.push((body, pool_blocks));
        *body_height = 0.0;
        *pool_height = 0.0;
        numbers.clear();
    };

    while let Some(block) = work.pop_front() {
        // Page-break marker: flush the current page (if any) and drop
        // the marker. Footnote pool follows the body that triggered it.
        if matches!(block.draw, super::block::BlockDraw::PageBreak) {
            if !current.is_empty() {
                flush(
                    &mut pages,
                    &mut current,
                    &mut current_pool_blocks,
                    &mut current_body_height,
                    &mut current_pool_height,
                    &mut current_numbers,
                );
            }
            continue;
        }
        let needed = block.height + block.space_after;

        // Speculatively recompute the pool if this block carries any
        // new footnote calls. (No new calls = no recomputation cost.)
        let mut new_calls = Vec::new();
        block.collect_footnote_numbers(&mut new_calls);
        let (speculative_pool_h, speculative_pool) = if new_calls.is_empty() {
            (current_pool_height, current_pool_blocks.clone())
        } else {
            let mut combined = current_numbers.clone();
            combined.extend_from_slice(&new_calls);
            pool_for(&combined)
        };

        if current_body_height + needed + speculative_pool_h <= page_budget {
            current_body_height += needed;
            current_pool_height = speculative_pool_h;
            current_pool_blocks = speculative_pool;
            current_numbers.extend(new_calls);
            current.push(block);
            continue;
        }

        // Doesn't fit. Try splitting against the body budget that
        // remains AFTER the (provisional) larger pool reserves space.
        let remaining = page_budget - current_body_height - speculative_pool_h;
        match block.try_split(remaining.max(0.0)) {
            SplitOutcome::Whole(b) => {
                current_body_height += b.height + b.space_after;
                current_pool_height = speculative_pool_h;
                current_pool_blocks = speculative_pool;
                current_numbers.extend(new_calls);
                current.push(b);
            }
            SplitOutcome::Split(head, mut tail) => {
                // The head's footnotes might be only a subset of
                // `new_calls` (because some calls live in lines that
                // landed in the tail). Refine: ask the head for the
                // numbers it actually carries, recompute pool with
                // those plus the existing page numbers, and use that.
                let mut head_calls = Vec::new();
                head.collect_footnote_numbers(&mut head_calls);
                let mut combined = current_numbers.clone();
                combined.extend_from_slice(&head_calls);
                let (pool_h, pool_blocks) = pool_for(&combined);
                current_pool_height = pool_h;
                current_pool_blocks = pool_blocks;
                current_numbers = combined;
                current.push(head);
                flush(
                    &mut pages,
                    &mut current,
                    &mut current_pool_blocks,
                    &mut current_body_height,
                    &mut current_pool_height,
                    &mut current_numbers,
                );
                // Refresh tail metrics since try_split may have
                // changed its height.
                tail.height = match &tail.draw {
                    super::block::BlockDraw::Text(s) => s.height(),
                    _ => tail.height,
                };
                work.push_front(tail);
            }
            SplitOutcome::NoFit(b) => {
                if !current.is_empty() {
                    flush(
                        &mut pages,
                        &mut current,
                        &mut current_pool_blocks,
                        &mut current_body_height,
                        &mut current_pool_height,
                        &mut current_numbers,
                    );
                    work.push_front(b);
                } else {
                    // Empty page and still doesn't fit even with no
                    // pool — place anyway (overflow into bottom).
                    let h = b.height + b.space_after;
                    let mut block_calls = Vec::new();
                    b.collect_footnote_numbers(&mut block_calls);
                    let (pool_h, pool_blocks) = if block_calls.is_empty() {
                        (0.0, Vec::new())
                    } else {
                        pool_for(&block_calls)
                    };
                    current_pool_height = pool_h;
                    current_pool_blocks = pool_blocks;
                    current_numbers.extend(block_calls);
                    current.push(b);
                    current_body_height = h;
                }
            }
        }
    }
    if !current.is_empty() {
        pages.push((current, current_pool_blocks));
    }
    pages
}

pub fn paginate(blocks: Vec<Block>, page_budget: f32) -> Vec<Vec<Block>> {
    let mut pages: Vec<Vec<Block>> = Vec::new();
    let mut current: Vec<Block> = Vec::new();
    let mut current_height = 0.0_f32;

    let mut work: VecDeque<Block> = blocks.into();

    while let Some(block) = work.pop_front() {
        let needed = block.height + block.space_after;

        // Quick fit.
        if current_height + needed <= page_budget {
            current_height += needed;
            current.push(block);
            continue;
        }

        // Doesn't fit as-is. Try to split.
        let remaining = page_budget - current_height;
        match block.try_split(remaining) {
            SplitOutcome::Whole(b) => {
                // Fit somehow after all (shouldn't normally happen).
                current_height += b.height + b.space_after;
                current.push(b);
            }
            SplitOutcome::Split(head, tail) => {
                current.push(head);
                pages.push(std::mem::take(&mut current));
                current_height = 0.0;
                work.push_front(tail);
            }
            SplitOutcome::NoFit(b) => {
                if !current.is_empty() {
                    // Flush current page; retry the block on a fresh page.
                    pages.push(std::mem::take(&mut current));
                    current_height = 0.0;
                    work.push_front(b);
                } else {
                    // Already on an empty page and the block still won't
                    // fit. Place it anyway (overflow into bottom margin).
                    let h = b.height + b.space_after;
                    current.push(b);
                    current_height = h;
                }
            }
        }
    }
    if !current.is_empty() {
        pages.push(current);
    }
    pages
}
