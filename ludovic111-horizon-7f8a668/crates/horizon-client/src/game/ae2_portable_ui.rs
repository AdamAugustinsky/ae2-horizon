//! Item-backed portable storage menus. The displayed slots are views; the
//! authoritative contents and energy remain in the player's actual cell item.
use super::{Game, Screen, ae2_fluids, ae2_interactions, ae2_lab};
use ae2_cell_core::{Action, Cell, ItemKey};
use horizon_client::{items::ItemStack, menu::Input, text::Text};
use horizon_world::nbt::Tag;

const PAGE: usize = 45;
#[derive(Clone, Default)]
pub(super) struct PortableDrag {
    active: bool,
    kind: i32,
    slots: Vec<usize>,
}

fn portable_fluid(s: &ItemStack) -> bool {
    s.item.starts_with("ae2:portable_fluid_cell_")
}
fn portable_item(s: &ItemStack) -> bool {
    s.item.starts_with("ae2:portable_item_cell_")
}
fn read(s: &ItemStack) -> Result<Cell, String> {
    if portable_fluid(s) {
        ae2_fluids::read_fluid_cell(s)
    } else if portable_item(s) {
        ae2_lab::read_cell(s)
    } else {
        Err("portable cell moved".into())
    }
}
fn write(s: &mut ItemStack, c: &Cell) -> Result<(), String> {
    if portable_fluid(s) {
        ae2_fluids::write_fluid_cell(s, c)
    } else {
        ae2_lab::write_cell(s, c)
    }
}
/// Source BasicCellInventory permits empty ordinary cells and rejects populated ones.
/// Unknown/invalid storage cells fail closed.
pub(super) use horizon_client::ae2_cell_codec::can_store_item;

fn nav(name: &str) -> ItemStack {
    let mut s = ItemStack::new("minecraft:arrow", 1);
    s.data_mut().custom_name = Some(name.into());
    s
}
impl Game<'_> {
    fn portable_stack(&self, source: usize) -> Result<ItemStack, String> {
        let stack = self
            .stack_in(source)
            .cloned()
            .ok_or("portable cell moved")?;
        if stack.count != 1 || !(portable_item(&stack) || portable_fluid(&stack)) {
            return Err("portable cell moved".into());
        }
        self.ae2_read_cell(&stack)?;
        ae2_interactions::portable_energy(&stack)?;
        Ok(stack)
    }
    fn portable_idle_step(&mut self, source: usize, buffer: &mut u8) -> Result<(), String> {
        if self.creative {
            return Ok(());
        }
        if *buffer > 0 {
            *buffer -= 1;
            return Ok(());
        }
        let mut stack = self.portable_stack(source)?;
        let got = ae2_interactions::portable_extract_energy(&mut stack, 5.0, false)?;
        *buffer = (got / 0.5).ceil().clamp(0.0, 10.0) as u8;
        *self.stack_in_mut(source) = Some(stack);
        Ok(())
    }
    fn portable_entries(&self, source: usize) -> Result<Vec<(ItemKey, u64)>, String> {
        let cell = self.ae2_read_cell(&self.portable_stack(source)?)?;
        Ok(cell.entries().map(|(k, n)| (k.clone(), *n)).collect())
    }
    fn portable_display(
        &self,
        source: usize,
        page: usize,
        index: usize,
        entries: &[(ItemKey, u64)],
    ) -> Option<ItemStack> {
        if index == 45 && page > 0 {
            return Some(nav("Previous page"));
        }
        if index == 53 && (page + 1) * PAGE < entries.len() {
            return Some(nav("Next page"));
        }
        if index >= PAGE {
            return None;
        }
        let (key, n) = entries.get(page * PAGE + index)?;
        let cell = self.stack_in(source)?;
        let mut stack = if portable_fluid(cell) {
            ItemStack::new(
                ae2_fluids::fluid_bucket(&key.item_id).unwrap_or("minecraft:bucket"),
                1,
            )
        } else {
            let mut stack = ae2_lab::prototype(key).ok()?;
            let max = self.res.items().stack_max(&stack);
            stack.count = (*n).min(u64::from(max)) as u32;
            stack
        };
        let text = if portable_fluid(cell) {
            format!("{}: {} mB", key.item_id, n)
        } else {
            format!("Stored: {n}")
        };
        let mut lore = match stack.data().and_then(|d| d.other("minecraft:lore")) {
            Some(Tag::List(v)) => v.clone(),
            _ => Vec::new(),
        };
        lore.push(Tag::String(text.as_str().into()));
        stack
            .data_mut()
            .set_other("minecraft:lore", Some(Tag::List(lore)));
        Some(stack)
    }
    fn portable_transfer_cost(
        &self,
        stack: &ItemStack,
        amount: u64,
        factor: f64,
    ) -> Result<u64, String> {
        let (power, _) = ae2_interactions::portable_energy(stack)?;
        let usage = horizon_client::ae2_common_config::gameplay().usage_multiplier;
        Ok(amount.min((power / usage * factor + 0.9).floor() as u64))
    }
    fn portable_debit(
        &self,
        stack: &mut ItemStack,
        amount: u64,
        factor: f64,
    ) -> Result<(), String> {
        let usage = horizon_client::ae2_common_config::gameplay().usage_multiplier;
        ae2_interactions::portable_extract_energy(stack, amount as f64 / factor * usage, false)?;
        Ok(())
    }
    fn portable_click_inner(
        &mut self,
        source: usize,
        page: &mut usize,
        slot: Option<usize>,
        button: i32,
        input: Input,
        drag: &mut PortableDrag,
    ) -> Result<(), String> {
        if input == Input::QuickCraft {
            let stage = button & 3;
            if stage == 0 {
                *drag = PortableDrag::default();
                let kind = (button >> 2) & 3;
                if self.carried.is_some()
                    && (kind == 0 || kind == 1 || (kind == 2 && self.creative))
                {
                    drag.active = true;
                    drag.kind = kind;
                }
            } else if stage == 1 && drag.active {
                if let (Some(slot), Some(held)) = (slot, self.carried.as_ref()) {
                    if slot < 36
                        && slot != source
                        && !drag.slots.contains(&slot)
                        && self.inventory[slot]
                            .as_ref()
                            .is_none_or(|item| item.same_item_same_components(held))
                        && (drag.kind == 2 || held.count as usize > drag.slots.len())
                    {
                        drag.slots.push(slot);
                    }
                }
            } else if stage == 2 && drag.active {
                if let Some(mut held) = self.carried.take() {
                    let original = held.count;
                    let max = self.res.items().stack_max(&held);
                    let each = match drag.kind {
                        0 => original / drag.slots.len().max(1) as u32,
                        1 => 1,
                        2 => max,
                        _ => 0,
                    };
                    for &slot in &drag.slots {
                        let previous = self.inventory[slot].as_ref().map_or(0, |v| v.count);
                        let n = each
                            .min(max.saturating_sub(previous))
                            .min(if drag.kind == 2 { u32::MAX } else { held.count });
                        if n == 0 {
                            continue;
                        }
                        if let Some(item) = self.inventory[slot].as_mut() {
                            item.count += n;
                        } else {
                            let mut item = held.clone();
                            item.count = n;
                            self.inventory[slot] = Some(item);
                        }
                        if drag.kind != 2 {
                            held.count -= n;
                        }
                    }
                    if held.count > 0 {
                        self.carried = Some(held);
                    }
                }
                *drag = PortableDrag::default();
            } else {
                *drag = PortableDrag::default();
            }
            return Ok(());
        }
        *drag = PortableDrag::default();
        let Some(slot) = slot else { return Ok(()) };
        let mut stack = self.portable_stack(source)?;
        let mut cell = self.ae2_read_cell(&stack)?;
        let fluid = portable_fluid(&stack);
        let entries: Vec<_> = cell.entries().map(|(k, n)| (k.clone(), *n)).collect();
        *page = (*page).min(entries.len().saturating_sub(1) / PAGE);
        if (300..354).contains(&slot) {
            if input == Input::Clone && self.creative && self.carried.is_none() && !fluid {
                if let Some((key, _)) = entries
                    .get(*page * PAGE + slot - 300)
                    .filter(|_| slot - 300 < PAGE)
                {
                    let mut item = ae2_lab::prototype(key)?;
                    item.count = self.res.items().stack_max(&item);
                    self.carried = Some(item);
                }
                return Ok(());
            }
            if !matches!(input, Input::Pickup | Input::QuickMove) || !(0..=1).contains(&button) {
                return Ok(());
            }
            let index = slot - 300;
            if index == 45 {
                *page = page.saturating_sub(1);
                return Ok(());
            }
            if index == 53 {
                if (*page + 1) * PAGE < entries.len() {
                    *page += 1;
                }
                return Ok(());
            }
            if index >= PAGE {
                return Ok(());
            }
            if fluid {
                if input == Input::QuickMove {
                    return Ok(());
                }
                return self
                    .portable_fluid_click(source, &mut stack, &mut cell, &entries, *page, index);
            }
            if input == Input::QuickMove {
                if self.carried.is_some() {
                    return Ok(());
                }
                let Some((key, available)) = entries.get(*page * PAGE + index) else {
                    return Ok(());
                };
                let mut item = ae2_lab::prototype(key)?;
                let max = self.res.items().stack_max(&item);
                let requested =
                    self.portable_transfer_cost(&stack, u64::from(max).min(*available), 1.0)?;
                if requested == 0 {
                    return Ok(());
                }
                item.count = requested as u32;
                let before = self.inventory.clone();
                let leftover = self.give(item.clone()).map_or(0, |s| u64::from(s.count));
                self.inventory = before.clone();
                let moved = cell.extract(key, requested - leftover, Action::Modulate);
                if moved == 0 {
                    return Ok(());
                }
                write(&mut stack, &cell)?;
                self.portable_debit(&mut stack, moved, 1.0)?;
                item.count = moved as u32;
                if self.give(item).is_some() {
                    self.inventory = before;
                    return Err("inventory changed during transfer".into());
                }
                *self.stack_in_mut(source) = Some(stack);
                return Ok(());
            }
            if let Some(carried) = self.carried.clone() {
                if !can_store_item(&carried) {
                    return Err("nonempty storage cells cannot be inserted".into());
                }
                let wanted = if button == 1 {
                    1
                } else {
                    u64::from(carried.count)
                };
                let limit = self.portable_transfer_cost(&stack, wanted, 1.0)?;
                let moved = self.ae2_cell_insert(
                    &mut cell,
                    &ae2_lab::key(&carried)?,
                    limit,
                    Action::Modulate,
                );
                if moved == 0 {
                    return Ok(());
                }
                write(&mut stack, &cell)?;
                self.portable_debit(&mut stack, moved, 1.0)?;
                let remain = carried.count - moved as u32;
                self.carried = (remain > 0).then_some(ItemStack {
                    count: remain,
                    ..carried
                });
                *self.stack_in_mut(source) = Some(stack);
            } else if let Some((key, available)) = entries.get(*page * PAGE + index) {
                let mut item = ae2_lab::prototype(key)?;
                let max = self.res.items().stack_max(&item);
                let available = u64::from(max).min(*available);
                let requested = if button == 1 {
                    available.div_ceil(2)
                } else {
                    available
                };
                let limit = self.portable_transfer_cost(&stack, requested, 1.0)?;
                let got = cell.extract(key, limit, Action::Modulate);
                if got == 0 {
                    return Ok(());
                }
                write(&mut stack, &cell)?;
                self.portable_debit(&mut stack, got, 1.0)?;
                item.count = got as u32;
                self.carried = Some(item);
                *self.stack_in_mut(source) = Some(stack);
            }
            return Ok(());
        }
        if slot >= 36 || slot == source {
            return Ok(());
        }
        if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            let target = button as usize;
            if target == source || target == slot {
                return Ok(());
            }
            let other = self.stack_in(target).cloned();
            let current = self.inventory[slot].take();
            self.inventory[slot] = other;
            *self.stack_in_mut(target) = current;
            return Ok(());
        }
        if input == Input::QuickMove && !fluid {
            if let Some(item) = self.inventory[slot].clone() {
                if !can_store_item(&item) {
                    return Err("nonempty storage cells cannot be inserted".into());
                }
                let limit = self.portable_transfer_cost(&stack, u64::from(item.count), 1.0)?;
                let moved =
                    self.ae2_cell_insert(&mut cell, &ae2_lab::key(&item)?, limit, Action::Modulate);
                if moved > 0 {
                    write(&mut stack, &cell)?;
                    self.portable_debit(&mut stack, moved, 1.0)?;
                    let left = item.count - moved as u32;
                    self.inventory[slot] = (left > 0).then_some(ItemStack {
                        count: left,
                        ..item
                    });
                    *self.stack_in_mut(source) = Some(stack);
                }
            }
            return Ok(());
        }
        if input != Input::Pickup || !(0..=1).contains(&button) {
            return Ok(());
        }
        let held = self.carried.take();
        let item = self.inventory[slot].take();
        match (held, item, button) {
            (None, None, _) => {}
            (None, Some(mut item), b) => {
                let n = if b == 1 {
                    item.count.div_ceil(2)
                } else {
                    item.count
                };
                item.count -= n;
                self.carried = Some(ItemStack {
                    count: n,
                    ..item.clone()
                });
                if item.count > 0 {
                    self.inventory[slot] = Some(item);
                }
            }
            (Some(mut held), None, b) => {
                let limit=self.res.items().stack_max(&held);
                let count=held.count.min(if b==1 {1}else{limit});
                self.inventory[slot]=Some(ItemStack{count,..held.clone()});held.count-=count;
                if held.count>0 {self.carried=Some(held);}
            }
            (Some(mut held), Some(mut item), b) if held.same_item_same_components(&item) => {
                let max = self.res.items().stack_max(&item);
                let n = max
                    .saturating_sub(item.count)
                    .min(if b == 1 { 1 } else { held.count });
                item.count += n;
                held.count -= n;
                self.inventory[slot] = Some(item);
                if held.count > 0 {
                    self.carried = Some(held);
                }
            }
            (Some(held), Some(item), _) if held.count<=self.res.items().stack_max(&held) => {
                self.inventory[slot] = Some(held);
                self.carried = Some(item);
            }
            (held, item, _) => {
                self.carried = held;
                self.inventory[slot] = item;
            }
        }
        Ok(())
    }
    fn portable_fluid_click(
        &mut self,
        source: usize,
        stack: &mut ItemStack,
        cell: &mut Cell,
        entries: &[(ItemKey, u64)],
        page: usize,
        index: usize,
    ) -> Result<(), String> {
        let Some(bucket) = self.carried.clone() else {
            return Ok(());
        };
        if bucket.count == 0 || !ae2_fluids::plain_bucket(&bucket) {
            return Err("use an unmodified bucket".into());
        }
        let (key, result, insert) = if let Some(id) = ae2_fluids::bucket_fluid(&bucket.item) {
            (ae2_fluids::fluid_key(id), "minecraft:bucket", true)
        } else {
            if bucket.item.as_ref() != "minecraft:bucket" {
                return Err("use an empty bucket".into());
            }
            let (key, amount) = entries
                .get(page * PAGE + index)
                .ok_or("fluid entry unavailable")?;
            if *amount < 1000 || !key.components.is_empty() {
                return Err("fluid cannot fill a vanilla bucket".into());
            }
            (
                key.clone(),
                ae2_fluids::fluid_bucket(&key.item_id).ok_or("unsupported fluid bucket")?,
                false,
            )
        };
        if self.portable_transfer_cost(stack, 1000, 125.0)? < 1000 {
            return Err("portable cell lacks power".into());
        }
        let moved = if insert {
            cell.insert(&key, 1000, Action::Simulate)
        } else {
            cell.extract(&key, 1000, Action::Simulate)
        };
        if moved != 1000 {
            return Err("portable cell cannot transfer a full bucket".into());
        }
        if self.res.items().get(result).is_none() {
            return Err("bucket item unavailable".into());
        }
        let before_inventory = self.inventory.clone();
        let before_cursor = self.carried.clone();
        if bucket.count == 1 {
            self.carried = Some(ItemStack::new(result, 1));
        } else {
            self.carried = Some(ItemStack {
                count: bucket.count - 1,
                ..bucket
            });
            if self.give(ItemStack::new(result, 1)).is_some() {
                self.inventory = before_inventory;
                self.carried = before_cursor;
                return Err("inventory full".into());
            }
        }
        let committed = if insert {
            cell.insert(&key, 1000, Action::Modulate)
        } else {
            cell.extract(&key, 1000, Action::Modulate)
        };
        if committed != 1000 {
            self.inventory = before_inventory;
            self.carried = before_cursor;
            return Err("cell changed during transfer".into());
        }
        if let Err(error) = write(stack, cell).and_then(|_| self.portable_debit(stack, 1000, 125.0))
        {
            self.inventory = before_inventory;
            self.carried = before_cursor;
            return Err(error);
        }
        *self.stack_in_mut(source) = Some(stack.clone());
        Ok(())
    }
    pub(super) fn ae2_try_open_portable(&mut self) -> bool {
        if self.remote.is_some()
            || self.lan_acting.is_some()
            || self.keys.contains(&super::KeyCode::ShiftLeft)
        {
            return false;
        }
        let source = self.held_slot();
        if self.portable_stack(source).is_err() {
            return false;
        }
        self.ae2_portable_slot = Some(source);
        self.ae2_portable_page = 0;
        self.ae2_portable_idle_buffer = 0;
        self.container = None;
        self.screen = Screen::Inventory;
        self.capture(false);
        true
    }
    pub(super) fn ae2_portable_open(&self) -> bool {
        self.remote.is_none()
            && self.ae2_portable_slot.is_some()
            && self.screen == Screen::Inventory
    }
    pub(super) fn ae2_portable_close(&mut self) {
        self.ae2_portable_slot = None;
        self.ae2_portable_page = 0;
        self.ae2_portable_idle_buffer = 0;
    }
    pub(super) fn ae2_portable_tick(&mut self) {
        if !self.ae2_portable_open() {
            return;
        }
        let Some(slot) = self.ae2_portable_slot else {
            return;
        };
        let mut buffer = self.ae2_portable_idle_buffer;
        if self.portable_idle_step(slot, &mut buffer).is_err() {
            self.ae2_portable_close()
        } else {
            self.ae2_portable_idle_buffer = buffer;
        }
    }
    pub(super) fn ae2_portable_slot_view(&self, slot: usize) -> Option<ItemStack> {
        if !self.ae2_portable_open() || !(300..354).contains(&slot) {
            return None;
        }
        let source = self.ae2_portable_slot?;
        let entries = self.portable_entries(source).ok()?;
        self.portable_display(source, self.ae2_portable_page, slot - 300, &entries)
    }
    pub(super) fn ae2_portable_click(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: Input,
    ) -> bool {
        if !self.ae2_portable_open() {
            return false;
        }
        let Some(source) = self.ae2_portable_slot else {
            return true;
        };
        let mut page = self.ae2_portable_page;
        let mut drag = std::mem::take(&mut self.ae2_portable_drag);
        if let Err(e) = self.portable_click_inner(source, &mut page, slot, button, input, &mut drag)
        {
            self.system_message(Text::literal(format!("Portable cell: {e}")));
        }
        self.ae2_portable_page = page;
        self.ae2_portable_drag = drag;
        true
    }
}

/// Per-connection menu metadata only; the item remains in guest inventory.
pub(super) struct GuestPortable {
    window: i32,
    state: i32,
    source: usize,
    page: usize,
    idle: u8,
    snapshot: ItemStack,
    drag: PortableDrag,
}
impl Game<'_> {
    /// Intercept serverbound UseItem before ordinary handling. `hand` is the
    /// protocol's main/offhand index. Returns false for non-portable items.
    pub(super) fn lan_open_ae2_portable(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        hand: i32,
    ) -> bool {
        let Some(guest) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return false;
        };
        if guest.spectator() || guest.sneaking {
            return false;
        }
        let source = if hand == 1 {
            super::shield::OFFHAND_SLOT
        } else {
            guest.selected
        };
        self.lan_open_ae2_portable_source(host, conn, source)
    }
    /// Inventory hotkeys locate their actual item without changing the held slot.
    pub(super) fn lan_open_ae2_portable_source(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        source: usize,
    ) -> bool {
        if !host.hub.guests.iter().any(|guest| guest.conn == conn && !guest.spectator())
            || (source >= 36 && source != super::shield::OFFHAND_SLOT)
        {
            return false;
        }
        let Some(snapshot) = self
            .as_guest(host, conn, |g| g.portable_stack(source).ok())
            .flatten()
        else {
            return false;
        };
        self.lan_ae2_portable_closed(host, conn, None);
        self.lan_ae2_bus_closed(host, conn, None);
        self.lan_ae2_network_tool_closed(host, conn, None);
        self.lan_ae2_anchor_closed(host, conn, None);
        self.lan_craft_confirm_closed(host, conn, None);
        self.lan_pattern_access_closed(host, conn, None);
        self.lan_ae2_terminal_closed(host, conn, None);
        self.lan_ae2_machine_closed(host, conn, None);
        self.lan_station_closed(host, conn, None);
        self.lan_crafting_closed(host, conn, None);
        self.guest_container_closed(host, conn);
        let Some(window) = host.hub.open_window(
            conn,
            "minecraft:generic_9x6",
            horizon_protocol::text::literal("Portable Cell"),
        ) else {
            return true;
        };
        Self::lan_mark_ae2_base_screen(host,conn,window);
        host.play.entry(conn).or_default().ae2_portable = Some(GuestPortable {
            window,
            state: 0,
            source,
            page: 0,
            idle: 0,
            snapshot,
            drag: PortableDrag::default(),
        });
        self.lan_send_ae2_portable(host, conn);
        true
    }
    fn lan_send_ae2_portable(&mut self, host: &mut super::lan::LanHost, conn: u32) {
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let Some((source, page)) = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_portable.as_ref())
            .map(|p| (p.source, p.page))
        else {
            return;
        };
        let rendered = self
            .as_guest(host, conn, |g| {
                let stack = g.portable_stack(source).ok()?;
                let entries = g.portable_entries(source).ok()?;
                let views = (0..54)
                    .map(|i| g.portable_display(source, page, i, &entries))
                    .collect::<Vec<_>>();
                Some((stack, views))
            })
            .flatten();
        let Some((snapshot, mut items)) = rendered else {
            let window = host
                .play
                .get(&conn)
                .and_then(|p| p.ae2_portable.as_ref())
                .map(|p| p.window);
            self.lan_ae2_portable_closed(host, conn, None);
            if let Some(container) = window {
                host.hub
                    .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
            }
            return;
        };
        let Some(guest) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        items.extend((9..36).chain(0..9).map(|i| guest.inventory[i].clone()));
        let carried = guest.carried.clone();
        let Some(menu) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.ae2_portable.as_mut())
        else {
            return;
        };
        menu.snapshot = snapshot;
        menu.state = (menu.state + 1) & 0x7fff;
        let (window, state) = (menu.window, menu.state);
        let items = items.iter().map(|s| host.hub.stack(s)).collect();
        let carried = host.hub.stack(&carried);
        host.hub.send_to(
            conn,
            &C::ContainerSetContent(play::ContainerSetContent {
                container: window,
                state,
                items,
                carried,
            }),
        );
    }
    pub(super) fn lan_ae2_portable_click(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        p: &horizon_protocol::packets::play::ContainerClick,
    ) -> bool {
        let Some((window, state, source, page, snapshot, drag)) = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_portable.as_ref())
            .filter(|v| v.window == p.container)
            .map(|v| {
                (
                    v.window,
                    v.state,
                    v.source,
                    v.page,
                    v.snapshot.clone(),
                    v.drag.clone(),
                )
            })
        else {
            return false;
        };
        if !host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .is_some_and(|g| !g.spectator())
        {
            self.lan_ae2_portable_closed(host, conn, None);
            return true;
        }
        let valid = self
            .as_guest(host, conn, |g| {
                g.portable_stack(source).ok().is_some_and(|s| s == snapshot)
            })
            .unwrap_or(false);
        // Virtual key views need revision protection; player and control
        // slots are fixed and can apply safely to current server state.
        if !valid || (p.state != state && (0..45).contains(&p.slot)) {
            self.lan_send_ae2_portable(host, conn);
            return true;
        }
        let slot = match p.slot {
            0..=53 => Some(300 + p.slot as usize),
            54..=80 => Some(9 + p.slot as usize - 54),
            81..=89 => Some(p.slot as usize - 81),
            _ => None,
        };
        let input = super::lan_crafting::input(p.input);
        let button = i32::from(p.button);
        let next = self.as_guest(host, conn, |g| {
            let mut page = page;
            let mut drag = drag;
            let _ = g.portable_click_inner(source, &mut page, slot, button, input, &mut drag);
            (page, drag)
        });
        if let Some((page, drag)) = next {
            if let Some(menu) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_portable.as_mut())
            {
                menu.page = page;
                menu.drag = drag;
            }
        }
        let _ = window;
        self.lan_send_ae2_portable(host, conn);
        true
    }
    /// Tick each active portable's idle drain, then refresh changed item views.
    pub(super) fn lan_refresh_ae2_portables(&mut self, host: &mut super::lan::LanHost) {
        let menus: Vec<_> = host
            .play
            .iter()
            .filter_map(|(&conn, p)| {
                if !host
                    .hub
                    .guests
                    .iter()
                    .any(|g| g.conn == conn && host.hub.here(g))
                {
                    return None;
                }
                p.ae2_portable
                    .as_ref()
                    .map(|v| (conn, v.source, v.idle, v.snapshot.clone(), v.window))
            })
            .collect();
        for (conn, source, mut idle, previous, window) in menus {
            let current = self
                .as_guest(host, conn, |g| {
                    g.portable_idle_step(source, &mut idle)?;
                    g.portable_stack(source)
                })
                .and_then(Result::ok);
            if current.is_none() {
                self.lan_ae2_portable_closed(host, conn, None);
                host.hub.send_to(
                    conn,
                    &horizon_protocol::packets::play::Clientbound::ContainerClose(
                        horizon_protocol::packets::play::ContainerClose { container: window },
                    ),
                );
                continue;
            }
            if let Some(menu) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_portable.as_mut())
            {
                menu.idle = idle;
            }
            if current.as_ref() != Some(&previous) {
                self.lan_send_ae2_portable(host, conn);
            }
        }
    }
    pub(super) fn lan_ae2_portable_closed(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let Some(_) = host.play.get_mut(&conn).and_then(|p| {
            p.ae2_portable
                .take_if(|v| window.is_none_or(|w| v.window == w))
        }) else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
            self.guest_carried_back(host, gi);
        }
        true
    }
}
