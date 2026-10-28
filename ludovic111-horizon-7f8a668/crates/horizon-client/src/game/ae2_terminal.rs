//! Local ME terminal: virtual views of authoritative drive-cell storage.
//! No display stack is ever stored in a block entity or treated as an item.
use super::{Game, ae2_lab};
use horizon_client::{items::ItemStack, menu::Input, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::nbt::Tag;
use std::sync::Arc;

const PAGE_ITEMS: usize = 45;
const PREV: usize = 45;
const NEXT: usize = 53;

fn navigation(label: &str) -> ItemStack {
    let mut stack = ItemStack {
        item: Arc::from("minecraft:arrow"),
        count: 1,
        damage: 0,
        extra: None,
    };
    stack.data_mut().custom_name = Some(label.into());
    stack
}
fn fluid_marker(key: &ae2_cell_core::ItemKey) -> ae2_cell_core::ItemKey {
    ae2_cell_core::ItemKey {
        item_id: format!("horizon:crafting_fluid/{}", key.item_id),
        components: key.components.clone(),
    }
}
fn marker_fluid(key: &ae2_cell_core::ItemKey) -> Option<ae2_cell_core::ItemKey> {
    Some(ae2_cell_core::ItemKey {
        item_id: key
            .item_id
            .strip_prefix("horizon:crafting_fluid/")?
            .to_owned(),
        components: key.components.clone(),
    })
}
fn terminal_filter_entries(
    entries: &[(ae2_cell_core::ItemKey, u64)],
    craftable: &[ae2_cell_core::ItemKey],
    sort: u8,
    descending: bool,
    filter: u8,
    search: &str,
    names: &horizon_client::creative::Names,
) -> Vec<(ae2_cell_core::ItemKey, u64)> {
    let query = search.to_lowercase();
    let mut out: Vec<_> = entries
        .iter()
        .filter(|(key, amount)| {
            (query.is_empty() || key.item_id.to_lowercase().contains(&query))
                && match filter {
                    1 => *amount > 0,
                    2 => craftable.contains(key),
                    _ => true,
                }
        })
        .cloned()
        .collect();
    if sort == 2 {
        // AE2's MOD mode groups by mod id, then case-insensitive display name.
        out.sort_by_cached_key(|(key, _)| {
            let id = key.item_id.strip_prefix("horizon:crafting_fluid/").unwrap_or(&key.item_id);
            let namespace = id.split_once(':').map_or("minecraft", |(namespace, _)| namespace);
            let name = if marker_fluid(key).is_none() {
                ae2_lab::prototype(key).ok().and_then(|item| item.custom_name().map(str::to_owned))
            } else { None }.unwrap_or_else(|| names.display_item(id));
            (namespace.to_lowercase(), name.to_lowercase())
        });
    } else if sort == 1 {
        out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    } else {
        out.sort_by(|a, b| a.0.cmp(&b.0));
    }
    if descending {
        out.reverse();
    }
    out
}
fn same(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item && a.damage == b.damage && a.extra == b.extra
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChestTerminalSettings {
    sort: u8,
    view: u8,
    descending: bool,
}

#[derive(Clone, Debug)]
struct PriorityEdit { value: i32, text: String, token:u64 }
// EditBox strips this control character, so ordinary terminal searches and
// anvil names cannot collide with the internal RenameItem envelope.
const PRIORITY_WIRE_PREFIX:&str="\u{1f}ae2p:";
pub(super) fn priority_transport_value(token:u64,text:&str)->String {format!("{PRIORITY_WIRE_PREFIX}{token}:{text}")}
fn priority_transport_parse(value:&str)->Option<(u64,&str)> {
    let (token,text)=value.strip_prefix(PRIORITY_WIRE_PREFIX)?.split_once(':')?;Some((token.parse().ok()?,text))
}
fn priority_transport_for(value:&str,expected:u64)->Option<&str> {
    let (token,text)=priority_transport_parse(value)?;(token==expected).then_some(text)
}
pub(super) fn priority_confirmable(text:&str)->bool {
    super::ae2_number_entry::number_value(text,1,i64::from(i32::MIN),i64::MAX,'.').is_some()
}
fn priority_step(text:&str,slot:i16,shift:bool)->Option<String> {
    if !(0..8).contains(&slot){return None;}
    let steps=if shift {[1i64,16,32,64]}else{[1i64,10,100,1000]};
    let delta=if slot<4 {steps[slot as usize]}else{-steps[(slot as usize)%4]};
    super::ae2_number_entry::number_step(text,delta,i64::from(i32::MIN),i64::MAX,'.')
}

fn chest_terminal_settings(machine: &horizon_client::ae2_world::Machine) -> ChestTerminalSettings {
    let setting = |key: &str| match machine.config.get(&key.into()) {
        Some(Tag::String(value)) => value.to_string().ok(),
        _ => None,
    };
    ChestTerminalSettings {
        sort: match setting("sort_by").as_deref() { Some("amount") => 1, Some("mod") => 2, _ => 0 },
        view: match setting("view_mode").as_deref() { Some("stored") => 1, Some("craftable") => 2, _ => 0 },
        descending: setting("sort_direction").as_deref() == Some("descending"),
    }
}

fn chest_terminal_setting_click(machine: &mut horizon_client::ae2_world::Machine, slot: i16, button: i8) -> bool {
    if machine.kind != horizon_client::ae2_world::MachineKind::Chest || !matches!(button, 0 | 1) { return false; }
    let current = chest_terminal_settings(machine);
    let step = if button == 0 { 1 } else { 2 };
    let (key, value) = match slot {
        46 => ("sort_by", ["name", "amount", "mod"][((current.sort + step) % 3) as usize]),
        47 => ("view_mode", ["all", "stored", "craftable"][((current.view + step) % 3) as usize]),
        51 => ("sort_direction", if current.descending { "ascending" } else { "descending" }),
        _ => return false,
    };
    machine.config.insert(key.into(), Tag::String(value.into()));
    true
}

pub(super) fn ae2_hand_menu_kind(kind:horizon_client::ae2_world::MachineKind)->bool {
    use horizon_client::ae2_world::MachineKind as K;
    matches!(kind,K::Drive|K::Chest|K::Terminal|K::VibrationChamber|K::Inscriber|K::Interface|K::PatternProvider|K::MolecularAssembler|K::CellWorkbench|K::QuantumLink|K::SpatialIoPort|K::IoPort|K::Condenser|K::WirelessAccessPoint|K::SkyStoneChest|K::SmoothSkyStoneChest)
}
impl Game<'_> {
    fn ae2_view_filtered(&self,entries:&[(ae2_cell_core::ItemKey,u64)],part:Option<super::ae2_parts::PartAddress>)->Vec<(ae2_cell_core::ItemKey,u64)> {
        entries.iter().filter(|(key,_)|self.ae2_view_permits(key,part,horizon_client::ae2_cell_codec::ViewCellScope::All)).cloned().collect()
    }
    fn ae2_view_permits(&self,key:&ae2_cell_core::ItemKey,part:Option<super::ae2_parts::PartAddress>,scope:horizon_client::ae2_cell_codec::ViewCellScope)->bool {
        let Some(part)=part else{return true;};let Some(machine)=self.ae2_part_machine(part).ok().flatten() else{return false;};
        let Ok(cells)=super::ae2_parts::ae2_terminal_view_cells(&machine) else{return false;};
        let fluid=marker_fluid(key);let key=fluid.as_ref().unwrap_or(key);let mut positive=false;let mut matched=false;
        for cell in cells.iter().flatten() {
            let Ok(m)=self.ae2_view_cell_match(cell,key,fluid.is_some(),scope) else{return false;};
            if !m.configured {continue;}if m.inverted && m.matches {return false;}if !m.inverted {positive=true;matched|=m.matches;}
        }
        !positive || matched
    }
    fn ae2_local_chest(&self, pos: (i32, i32, i32)) -> bool {
        matches!(self.world.block_entities.get(&pos), Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind == horizon_client::ae2_world::MachineKind::Chest)
    }
    fn chest_terminal_read(&self, pos: (i32, i32, i32)) -> Option<(u64, ChestTerminalSettings)> {
        let identity = self.world.block_entities.incarnation(&pos)?;
        let horizon_client::containers::BlockEntity::Ae2(machine) = self.world.block_entities.get(&pos)? else { return None; };
        (machine.kind == horizon_client::ae2_world::MachineKind::Chest).then(|| (identity, chest_terminal_settings(machine)))
    }
    fn chest_terminal_write(&mut self, pos: (i32, i32, i32), identity: u64, slot: i16, button: i8) -> Option<ChestTerminalSettings> {
        if self.world.block_entities.incarnation(&pos) != Some(identity) { return None; }
        let horizon_client::containers::BlockEntity::Ae2(machine) = self.world.block_entities.get_mut(&pos)? else { return None; };
        if !chest_terminal_setting_click(machine, slot, button) { return None; }
        let settings = chest_terminal_settings(machine);
        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        Some(settings)
    }
    fn ae2_terminal_powered(&self, pos: (i32, i32, i32)) -> bool {
        self.ae2_terminal_online(pos)
            || matches!(self.world.block_entities.get(&pos), Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind == horizon_client::ae2_world::MachineKind::Chest && m.energy > 1.0)
    }
    /// ME chest has its own cell, even when connected to a larger grid.
    /// Stage cell and power changes before committing either of them.
    fn ae2_chest_transfer(
        &mut self,
        pos: (i32, i32, i32),
        key: &ae2_cell_core::ItemKey,
        limit: u64,
        insert: bool,
    ) -> Result<u64, String> {
        use ae2_cell_core::Action;
        use horizon_client::containers::BlockEntity;
        let Some(BlockEntity::Ae2(m)) = self.world.block_entities.get(&pos) else {
            return Err("missing ME chest".into());
        };
        let mut stack = match m.slots.first().and_then(Option::as_ref) {
            Some(s) => s.clone(),
            None => return Ok(0),
        };
        let battery = m.energy.max(0.0);
        let fluid = marker_fluid(key);
        let physical_key = fluid.as_ref().unwrap_or(key);
        let creative = super::ae2_storage_devices::creative_cell_keys(&stack).ok();
        let creative_accepts = creative.as_ref().is_some_and(|keys| {
            keys.iter().any(|entry| match (entry, fluid.is_some()) {
                (super::ae2_storage_devices::CreativeCellKey::Item(k), false) => k == physical_key,
                (super::ae2_storage_devices::CreativeCellKey::Fluid(k), true) => k == physical_key,
                _ => false,
            })
        });
        let is_fluid = fluid.is_some();
        let mut cell = if creative.is_some() {
            None
        } else if is_fluid {
            Some(super::ae2_fluids::read_fluid_cell(&stack)?)
        } else {
            Some(ae2_lab::read_cell(&stack)?)
        };
        let possible = if let Some(cell) = cell.as_mut() {
            if insert {
                self.ae2_cell_insert(cell, physical_key, limit, Action::Simulate)
            } else {
                cell.extract(physical_key, limit, Action::Simulate)
            }
        } else if creative_accepts {
            limit
        } else {
            0
        };
        let per_unit = if is_fluid { 1.0 / 125.0 } else { 1.0 };
        let grid = self.ae2_take_power(pos, possible as f64 * per_unit, true);
        if is_fluid && (possible < limit || grid + battery < limit as f64 * per_unit) {
            return Ok(0);
        }
        let amount = if is_fluid {
            limit
        } else {
            possible.min(((grid + battery + 0.9) / per_unit).floor() as u64)
        };
        if amount == 0 {
            return Ok(0);
        }
        let moved = if let Some(cell) = cell.as_mut() {
            if insert {
                self.ae2_cell_insert(cell, physical_key, amount, Action::Modulate)
            } else {
                cell.extract(physical_key, amount, Action::Modulate)
            }
        } else {
            amount
        };
        if let Some(cell) = cell.as_ref() {
            if is_fluid {
                super::ae2_fluids::write_fluid_cell(&mut stack, cell)?;
            } else {
                ae2_lab::write_cell(&mut stack, cell)?;
            }
        }
        let paid_grid = self.ae2_take_power(pos, moved as f64 * per_unit, false);
        let Some(BlockEntity::Ae2(m)) = self.world.block_entities.get_mut(&pos) else {
            return Err("ME chest disappeared".into());
        };
        m.energy = (m.energy - (moved as f64 * per_unit - paid_grid).max(0.0)).max(0.0);
        m.slots[0] = Some(stack);
        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        Ok(moved)
    }
    fn ae2_terminal_insert(
        &mut self,
        pos: (i32, i32, i32),
        stack: &ItemStack,
        amount: u64,
    ) -> Result<u64, String> {
        if self.ae2_local_chest(pos) {
            self.ae2_chest_transfer(
                pos,
                &ae2_lab::key(stack)?,
                amount.min(stack.count as u64),
                true,
            )
        } else {
            self.ae2_grid_insert(pos, stack, amount)
        }
    }
    fn ae2_terminal_extract(
        &mut self,
        pos: (i32, i32, i32),
        key: &ae2_cell_core::ItemKey,
        amount: u64,
    ) -> Result<Option<ItemStack>, String> {
        if !self.ae2_local_chest(pos) {
            return self.ae2_grid_extract(pos, key, amount);
        }
        let mut item = ae2_lab::prototype(key)?;
        let moved = self.ae2_chest_transfer(pos, key, amount, false)?;
        if moved == 0 {
            return Ok(None);
        }
        item.count = moved as u32;
        Ok(Some(item))
    }
    fn ae2_terminal_entries(
        &self,
        pos: (i32, i32, i32),
    ) -> Result<Vec<(ae2_cell_core::ItemKey, u64)>, String> {
        if !self.ae2_terminal_powered(pos) {
            return Err("terminal is offline: check power and channels".into());
        }
        if self.ae2_local_chest(pos) {
            let Some(horizon_client::containers::BlockEntity::Ae2(m)) =
                self.world.block_entities.get(&pos)
            else {
                return Ok(Vec::new());
            };
            let Some(stack) = m.slots.first().and_then(Option::as_ref) else {
                return Ok(Vec::new());
            };
            if let Ok(keys) = super::ae2_storage_devices::creative_cell_keys(stack) {
                return Ok(keys
                    .into_iter()
                    .map(|entry| match entry {
                        super::ae2_storage_devices::CreativeCellKey::Item(key) => {
                            (key, i32::MAX as u64)
                        }
                        super::ae2_storage_devices::CreativeCellKey::Fluid(key) => {
                            (fluid_marker(&key), i32::MAX as u64)
                        }
                    })
                    .collect());
            }
            if stack.item.starts_with("ae2:fluid_storage_cell_") {
                return Ok(super::ae2_fluids::read_fluid_cell(stack)?
                    .entries()
                    .map(|(k, n)| (fluid_marker(k), *n))
                    .collect());
            }
            return Ok(ae2_lab::read_cell(stack)?
                .entries()
                .map(|(k, n)| (k.clone(), *n))
                .collect());
        }
        let mut entries = self.ae2_grid_inventory(pos)?;
        for key in self.ae2_craftable_outputs(pos) {
            if !entries.iter().any(|(existing, _)| *existing == key) {
                entries.push((key, 0));
            }
        }
        for key in self.ae2_craftable_fluids(pos) {
            let key = fluid_marker(&key);
            if !entries.iter().any(|(existing, _)| *existing == key) {
                entries.push((key, 0));
            }
        }
        Ok(entries)
    }
    pub(super) fn ae2_cpu_label(&self, address: super::ae2_global::GridAddress) -> String {
        self.ae2_address_parts(address)
            .map(|(dim, p)| format!("{} ({},{},{})", dim.id(), p.0, p.1, p.2))
            .unwrap_or_else(|| "unknown CPU".into())
    }
    fn ae2_job_button(&self, pos: (i32, i32, i32), selected: usize) -> Option<ItemStack> {
        let cpus=self.ae2_terminal_status_cpus(pos);
        let jobs = self.ae2_recursive_jobs(pos);
        let label=if cpus.is_empty() {"Crafting status: no CPUs".to_owned()} else {format!("Crafting {} {}/{}: click to view",if jobs.is_empty() {"status"} else {"job"},selected % cpus.len()+1,cpus.len())};
        let mut icon = navigation(&label);
        let lines: Vec<_> = jobs
            .iter()
            .map(|(p, pending, flight, cancelled)| {
                Tag::String(
                    format!(
                        "CPU {}: {} pending, {} in flight{}",
                        self.ae2_cpu_label(*p),
                        pending,
                        flight,
                        if *cancelled { " (cancelling)" } else { "" }
                    )
                    .as_str()
                    .into(),
                )
            })
            .collect();
        icon.data_mut()
            .set_other("minecraft:lore", Some(Tag::List(lines)));
        Some(icon)
    }
    pub(super) fn ae2_terminal_open(&self) -> bool {
        self.remote.is_none() && self.container.is_some_and(|p| self.ae2_has_terminal(p))
    }
    pub(super) fn ae2_open_terminal(&mut self, pos: (i32, i32, i32)) -> bool {
        if !self.ae2_has_terminal(pos) {
            return false;
        }
        self.ae2_terminal_page = 0;
        self.container = Some(pos);
        true
    }
    /// The displayed stack is derived fresh from cells and used solely for rendering.
    pub(super) fn ae2_terminal_slot_view(&self, slot: usize) -> Option<ItemStack> {
        if !self.ae2_terminal_open() || !(300..354).contains(&slot) {
            return None;
        }
        let pos = self.container?;
        if slot == 349 {
            return self.ae2_job_button(pos, 0);
        }
        let entries = self.ae2_terminal_entries(pos).ok()?;
        self.ae2_terminal_display(slot - 300, self.ae2_terminal_page, &entries, PAGE_ITEMS)
    }
    fn ae2_terminal_display(
        &self,
        index: usize,
        page: usize,
        entries: &[(ae2_cell_core::ItemKey, u64)],
        page_size: usize,
    ) -> Option<ItemStack> {
        let page = page.min(entries.len().saturating_sub(1) / page_size);
        if index == PREV && page > 0 {
            return Some(navigation("Previous page"));
        }
        if index == NEXT && (page + 1) * page_size < entries.len() {
            return Some(navigation("Next page"));
        }
        if index >= page_size {
            return None;
        }
        let (key, amount) = entries.get(page * page_size + index)?;
        if let Some(fluid) = marker_fluid(key) {
            let id = super::ae2_fluids::fluid_bucket(&fluid.item_id).unwrap_or("minecraft:bucket");
            let mut icon = ItemStack::new(id, 1);
            icon.data_mut().custom_name = Some(if *amount == 0 {
                format!("Craft 1000 mB {}", fluid.item_id)
            } else {
                format!("{}: {} mB", fluid.item_id, amount)
            });
            return Some(icon);
        }
        let mut stack = ae2_lab::prototype(key).ok()?;
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
            .max(1);
        stack.count = (*amount).max(1).min(u64::from(max)) as u32;
        let mut lore = match stack.data().and_then(|d| d.other("minecraft:lore")) {
            Some(Tag::List(v)) => v.clone(),
            _ => Vec::new(),
        };
        let label = if *amount == 0 {
            "Craftable: click to request".to_owned()
        } else {
            format!("Stored: {amount}")
        };
        lore.push(Tag::String(label.as_str().into()));
        stack
            .data_mut()
            .set_other("minecraft:lore", Some(Tag::List(lore)));
        Some(stack)
    }
    /// Consume all menu inputs when terminal is open: the generic chest menu
    /// must never see virtual slots, even for unsupported click gestures.
    pub(super) fn ae2_terminal_click(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: Input,
    ) -> bool {
        if !self.ae2_terminal_open() {
            return false;
        }
        let result = self.ae2_terminal_click_inner(slot, button, input, None, false, 1, PAGE_ITEMS);
        if let Err(message) = result {
            self.system_message(Text::literal(format!("ME terminal: {message}")));
        }
        true
    }
    fn ae2_terminal_click_inner(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: Input,
        entries_override: Option<&[(ae2_cell_core::ItemKey, u64)]>,
        wireless: bool,
        craft_amount: u64,
        page_size: usize,
    ) -> Result<(), String> {
        let pos = self.container.ok_or("terminal closed")?;
        if !wireless && !self.ae2_has_terminal(pos) {
            return Err("terminal disconnected".into());
        }
        let Some(slot) = slot else {
            return Ok(());
        };
        if (300..354).contains(&slot) {
            if !matches!(input, Input::Pickup | Input::QuickMove) || !(0..=1).contains(&button) {
                return Ok(());
            }
            let index = slot - 300;
            if index == 49 {
                let jobs = self.ae2_recursive_jobs(pos);
                if let Some((cpu, pending, flight, cancelled)) = jobs.first().copied() {
                    if button == 1 {
                        self.ae2_recursive_cancel(cpu)?;
                        self.system_message(Text::literal(
                            "Crafting cancellation requested; in-flight work will be reconciled",
                        ));
                    } else {
                        self.system_message(Text::literal(format!(
                            "CPU {}: {} pending, {} in flight{}",
                            self.ae2_cpu_label(cpu),
                            pending,
                            flight,
                            if cancelled { " (cancelling)" } else { "" }
                        )));
                    }
                }
                return Ok(());
            }
            let entries = match entries_override {
                Some(v) => v.to_vec(),
                None => self.ae2_terminal_entries(pos)?,
            };
            self.ae2_terminal_page = self
                .ae2_terminal_page
                .min(entries.len().saturating_sub(1) / page_size);
            if index == PREV {
                self.ae2_terminal_page = self.ae2_terminal_page.saturating_sub(1);
                return Ok(());
            }
            if index == NEXT {
                if (self.ae2_terminal_page + 1) * page_size < entries.len() {
                    self.ae2_terminal_page += 1;
                }
                return Ok(());
            }
            if index >= page_size {
                return Ok(());
            }
            if self.ae2_local_chest(pos) {
                let selected = entries.get(self.ae2_terminal_page * page_size + index);
                let supports_fluid = matches!(self.world.block_entities.get(&pos), Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.slots.first().and_then(Option::as_ref).is_some_and(|stack| stack.item.starts_with("ae2:fluid_storage_cell_") || super::ae2_storage_devices::creative_cell_keys(stack).is_ok_and(|keys| keys.iter().any(|key| matches!(key, super::ae2_storage_devices::CreativeCellKey::Fluid(_))))));
                if input == Input::Pickup && supports_fluid {
                    if let Some(cursor) = self
                        .carried
                        .clone()
                        .filter(|s| s.count == 1 && s.damage == 0 && s.data().is_none())
                    {
                        if let Some(fluid) = super::ae2_fluids::bucket_fluid(&cursor.item) {
                            let marker = fluid_marker(&super::ae2_fluids::fluid_key(fluid));
                            if self.ae2_chest_transfer(pos, &marker, 1000, true)? == 1000 {
                                self.carried = Some(ItemStack::new("minecraft:bucket", 1));
                            }
                            return Ok(());
                        }
                        if cursor.item.as_ref() == "minecraft:bucket" {
                            if let Some((key, amount)) = selected {
                                if let Some(fluid) = marker_fluid(key) {
                                    if *amount >= 1000
                                        && fluid == super::ae2_fluids::fluid_key(&fluid.item_id)
                                    {
                                        if let Some(bucket) =
                                            super::ae2_fluids::fluid_bucket(&fluid.item_id)
                                        {
                                            if self.ae2_chest_transfer(pos, key, 1000, false)?
                                                == 1000
                                            {
                                                self.carried = Some(ItemStack::new(bucket, 1));
                                            }
                                        }
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                if selected.is_some_and(|(key, _)| marker_fluid(key).is_some()) {
                    return Ok(());
                }
            }
            if input == Input::QuickMove {
                if let Some((key, _)) = entries.get(self.ae2_terminal_page * page_size + index) {
                    if marker_fluid(key).is_some()
                        || self.ae2_craftable_outputs(pos).iter().any(|k| k == key)
                    {
                        let cpu = if let Some(fluid) = marker_fluid(key) {
                            self.ae2_begin_recursive_fluid_craft(
                                pos,
                                &fluid,
                                craft_amount
                                    .checked_mul(1000)
                                    .ok_or("craft amount overflow")?,
                            )?
                        } else {
                            self.ae2_begin_recursive_craft(pos, key, craft_amount)?
                        };
                        self.system_message(Text::literal(format!(
                            "Crafting started on CPU {}",
                            self.ae2_cpu_label(cpu)
                        )));
                    }
                }
                return Ok(());
            }
            if let Some(carried) = self.carried.clone() {
                let requested = if button == 1 {
                    1
                } else {
                    u64::from(carried.count)
                };
                let moved = self.ae2_terminal_insert(pos, &carried, requested)?;
                if moved > requested || moved > u64::from(carried.count) {
                    return Err("invalid grid insert count".into());
                }
                let left = carried.count - moved as u32;
                self.carried = (left > 0).then_some(ItemStack {
                    count: left,
                    ..carried
                });
            } else if let Some((key, count)) =
                entries.get(self.ae2_terminal_page * page_size + index)
            {
                if *count == 0 {
                    let cpu = if let Some(fluid) = marker_fluid(key) {
                        self.ae2_begin_recursive_fluid_craft(
                            pos,
                            &fluid,
                            craft_amount
                                .checked_mul(1000)
                                .ok_or("craft amount overflow")?,
                        )?
                    } else {
                        self.ae2_begin_recursive_craft(pos, key, craft_amount)?
                    };
                    self.system_message(Text::literal(format!(
                        "Crafting started on CPU {}",
                        self.ae2_cpu_label(cpu)
                    )));
                    return Ok(());
                }
                let exemplar = ae2_lab::prototype(key)?;
                let max = self
                    .res
                    .items()
                    .get(&exemplar.item)
                    .map_or(64, |i| i.max_stack)
                    .max(1);
                let amount = if button == 1 {
                    u64::from(max).min(*count).div_ceil(2)
                } else {
                    u64::from(max).min(*count)
                };
                if amount > 0 {
                    self.carried = self.ae2_terminal_extract(pos, key, amount)?;
                }
            }
            return Ok(());
        }
        if slot >= 36 {
            return Ok(());
        }
        if input == Input::QuickMove {
            if !self.ae2_terminal_powered(pos) {
                return Err("terminal is offline: check power and channels".into());
            }
            if let Some(stack) = self.inventory[slot].clone() {
                let moved = self.ae2_terminal_insert(pos, &stack, u64::from(stack.count))?;
                if moved > u64::from(stack.count) {
                    return Err("invalid grid insert count".into());
                }
                let left = stack.count - moved as u32;
                self.inventory[slot] = (left > 0).then_some(ItemStack {
                    count: left,
                    ..stack
                });
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
            (None, Some(item), 0) => {
                self.carried = Some(item);
            }
            (None, Some(mut item), 1) => {
                let n = item.count.div_ceil(2);
                item.count -= n;
                self.carried = Some(ItemStack {
                    count: n,
                    ..item.clone()
                });
                if item.count > 0 {
                    self.inventory[slot] = Some(item);
                }
            }
            (Some(held), None, 0) => self.inventory[slot] = Some(held),
            (Some(mut held), None, 1) => {
                self.inventory[slot] = Some(ItemStack {
                    count: 1,
                    ..held.clone()
                });
                held.count -= 1;
                if held.count > 0 {
                    self.carried = Some(held);
                }
            }
            (Some(mut held), Some(mut item), b) if same(&held, &item) => {
                let max = self
                    .res
                    .items()
                    .get(&item.item)
                    .map_or(64, |i| i.max_stack)
                    .max(1);
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
            (Some(held), Some(item), _) => {
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
}

/// Ephemeral per-connection menu state. It contains no item storage.
struct ViewCellDrag { carried:ItemStack, slots:Vec<usize> }

// Inventory.getItem uses 0..35 for the main inventory, 36..39 for armor
// (feet through head), and 40 for the off hand. The locator remains tied
// to this slot even if another Network Tool replaces the original stack.
fn toolbox_owner(g: &horizon_client::lan::guest::Guest) -> Option<usize> {
    (0..=40).find(|&i| toolbox_stack(g, i).is_some_and(|s|
        s.item.as_ref() == horizon_client::ae2_network_tool::TOOL))
}
fn toolbox_stack(g: &horizon_client::lan::guest::Guest, i: usize) -> Option<&ItemStack> {
    match i {
        0..=35 => g.inventory.get(i)?.as_ref(),
        36..=39 => g.armor.get(39 - i)?.as_ref(),
        40 => g.offhand.as_ref(),
        _ => None,
    }
}
fn toolbox_stack_mut(g: &mut horizon_client::lan::guest::Guest, i: usize) -> Option<&mut ItemStack> {
    match i {
        0..=35 => g.inventory.get_mut(i)?.as_mut(),
        36..=39 => g.armor.get_mut(39 - i)?.as_mut(),
        40 => g.offhand.as_mut(),
        _ => None,
    }
}
// Menu::set_slot caps main-container entries. Refuse an invalid pre-existing
// overstack before staging any of the nine slots so an unrelated click cannot
// silently discard the excess.
fn toolbox_staging_safe(
    pocket: &horizon_client::ae2_network_tool::Inventory,
    max: impl Fn(&ItemStack) -> u32,
) -> bool {
    pocket.slots.iter().flatten().all(|s| s.count > 0 && s.count <= max(s).max(1).min(99))
}

/// A quick-craft gesture is pending input, never an owner of the stacks.
struct CraftMatrixDrag {
    identity: (u64, [i32; 4]),
    mode: i32,
    creative: bool,
    carried: ItemStack,
    inventory: Vec<Option<ItemStack>>,
    matrix: Vec<Option<ItemStack>>,
    slots: Vec<i32>,
}

/// A wireless drag reserves no items: it is replayed only after the source,
/// cursor and other player slots still match this pending gesture.
struct WirelessDrag {
    source: usize,
    identity: horizon_client::ae2_stack_identity::StackRef,
    mode: i32,
    creative: bool,
    carried: ItemStack,
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    matrix: Option<Vec<Option<ItemStack>>>,
    slots: Vec<i32>,
}
fn wireless_unchanged_inventory(
    source: usize, before: &[Option<ItemStack>], after: &[Option<ItemStack>],
    old_offhand: &Option<ItemStack>, new_offhand: &Option<ItemStack>,
) -> bool {
    before.len() == after.len()
        && before.iter().zip(after).enumerate().all(|(i, (a, b))| i == source || a == b)
        && (source == super::shield::OFFHAND_SLOT || old_offhand == new_offhand)
}

#[derive(Clone)]
enum CraftOwner {
    Part(super::ae2_parts::PartAddress),
    Wireless(super::ae2_wireless::WirelessContext),
}
fn craft_owner(
    part: Option<super::ae2_parts::PartAddress>,
    wireless: Option<&super::ae2_wireless::WirelessContext>,
) -> Option<CraftOwner> {
    part.map(CraftOwner::Part).or_else(|| wireless
        .filter(|ctx| ctx.item == "ae2:wireless_crafting_terminal")
        .cloned().map(CraftOwner::Wireless))
}

struct ToolboxDrag {
    owner: usize,
    tool: ItemStack,
    wireless: Option<(usize, horizon_client::ae2_stack_identity::StackRef)>,
    identity: Option<(u64, [i32; 4])>,
    mode: i32,
    creative: bool,
    carried: ItemStack,
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    matrix: Vec<Option<ItemStack>>,
    slots: Vec<i32>,
}

/// A remote wireless terminal can expose a toolbox without a view-cell panel.
pub(super) fn terminal_visible_slots(view_cells: bool, toolbox: bool) -> Vec<usize> {
    let mut slots: Vec<usize> = (0..36).chain(300..354).collect();
    if view_cells { slots.extend(360..365); }
    if toolbox { slots.extend(365..374); }
    slots
}

// Two independent item hosts may both be protected by one terminal menu.
fn wireless_toolbox_swap_refused(source: usize, toolbox: Option<usize>, button: i8) -> bool {
    source == 40 || usize::try_from(button).ok() == Some(source)
        || toolbox.is_some_and(|owner| owner == 40 || (owner < 9 && usize::try_from(button).ok() == Some(owner)))
}
fn toolbox_pickup_locked(index: usize, owner: usize, wireless_source: Option<usize>) -> bool {
    (9..45).contains(&index) && (index == 9 + owner || wireless_source.is_some_and(|source| index == 9 + source))
}
fn toolbox_drag_slot(slot: i16, matrix: bool, owner: usize, wireless_source: Option<usize>) -> Option<i32> {
    match slot {
        95..=103 => Some(i32::from(slot - 95)),
        27..=35 if matrix => Some(i32::from(slot - 27 + 9)),
        54..=80 => {let source=(slot-54+9) as usize;(source != owner && wireless_source != Some(source)).then_some(i32::from(slot-54+18))},
        81..=89 => {let source=(slot-81) as usize;(source != owner && wireless_source != Some(source)).then_some(i32::from(slot-81+45))},
        _ => None,
    }
}

/// Convert only physical terminal slots to the native crafting menu.
fn craft_drag_menu_slot(slot: i16) -> Option<i32> {
    match slot {
        27..=35 => Some(i32::from(slot - 27 + 1)),
        54..=80 => Some(i32::from(slot - 54 + 10)),
        81..=89 => Some(i32::from(slot - 81 + 37)),
        _ => None,
    }
}

pub(super) struct GuestTerminal {
    window: i32,
    state: i32,
    pos: (i32, i32, i32),
    wireless: Option<super::ae2_wireless::WirelessContext>,
    wireless_connected: bool,
    owner_dim: String,
    craft_part: Option<super::ae2_parts::PartAddress>,
    encoding_part: Option<super::ae2_parts::PartAddress>,
    view_part: Option<super::ae2_parts::PartAddress>,
    view_identity: Option<(u64,[i32;4])>,
    chest_identity: Option<u64>,
    chest_revisions: std::collections::VecDeque<(i32, ChestTerminalSettings)>,
    priority_edit: Option<PriorityEdit>,
    toolbox_owner: Option<usize>,
    toolbox_snapshot: Option<ItemStack>,
    player_snapshot: Vec<Option<ItemStack>>,
    view_drag: Option<ViewCellDrag>,
    craft_drag: Option<CraftMatrixDrag>,
    wireless_drag: Option<WirelessDrag>,
    toolbox_drag: Option<ToolboxDrag>,
    encoding_identity: Option<(u64,[i32;4])>,
    encoding_snapshot: Option<horizon_client::ae2_world::Machine>,
    encoding_revisions: std::collections::VecDeque<(i32, Option<horizon_client::ae2_world::Machine>)>,
    craft_snapshot: Vec<Option<ItemStack>>,
    page: usize,
    job_index: usize,
    craft_amount: u64,
    craft_amount_edit: bool,
    encoding: bool,
    pattern_grid: Vec<Option<ItemStack>>,
    substitutes: bool,
    fluid_substitutes: bool,
    processing: bool,
    special_mode:u8,
    stone_recipe:Option<String>,
    processing_outputs: Vec<Option<ItemStack>>,
    fluid_mode: bool,
    processing_page: usize,
    selected_fluid: usize,
    sort_mode: u8,
    sort_descending: bool,
    filter_mode: u8,
    search: String,
    search_edit: bool,
    fluid_amount: u64,
    fluid_inputs: Vec<Option<super::ae2_processing::PatternFluid>>,
    fluid_outputs: Vec<Option<super::ae2_processing::PatternFluid>>,
    snapshot: Vec<(ae2_cell_core::ItemKey, u64)>,
    displayed_snapshot: Vec<(ae2_cell_core::ItemKey, u64)>,
}
fn encoding_revision_matches(
    revisions:&std::collections::VecDeque<(i32,Option<horizon_client::ae2_world::Machine>)>,
    state:i32,current:Option<&horizon_client::ae2_world::Machine>,
)->bool {
    revisions.iter().rev().find(|(revision,_)|*revision==state)
        .and_then(|(_,shown)|Some((super::ae2_encoding::EncodingState::load(shown.as_ref()?).ok()?,super::ae2_encoding::EncodingState::load(current?).ok()?)))
        .is_some_and(|(shown,current)|shown==current)
}
fn chest_revision_matches(revisions:&std::collections::VecDeque<(i32,ChestTerminalSettings)>, state:i32, current:ChestTerminalSettings)->bool {
    revisions.iter().rev().find(|(revision,_)|*revision==state).is_some_and(|(_,shown)|*shown==current)
}

impl Game<'_> {
    pub(super) fn lan_ae2_wireless_source_open(
        &self, host: &super::lan::LanHost, conn: u32, source: usize,
    ) -> bool {
        host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.wireless.as_ref()).is_some_and(|ctx| ctx.source == source)
    }
    fn lan_craft_matrix_read(
        &self, host: &super::lan::LanHost, conn: u32, owner: &CraftOwner,
    ) -> Result<horizon_client::ae2_world::Machine, String> {
        use horizon_client::ae2_world::{Machine, MachineKind};
        match owner {
            CraftOwner::Part(part) => self.ae2_part_machine(*part)?.ok_or("missing crafting terminal".into()),
            CraftOwner::Wireless(ctx) => {
                if !self.ae2_wireless_source_valid(host, conn, ctx) { return Err("wireless crafting terminal moved".into()); }
                let stack = host.hub.guests.iter().find(|g| g.conn == conn)
                    .and_then(|g| toolbox_stack(g, ctx.source)).ok_or("wireless crafting terminal moved")?;
                let mut machine = Machine::new(MachineKind::CraftingTerminal);
                machine.slots = super::ae2_wireless::crafting_grid(stack)?;
                Ok(machine)
            }
        }
    }
    fn lan_craft_matrix_store(
        &mut self, host: &mut super::lan::LanHost, conn: u32, owner: &CraftOwner,
        machine: &horizon_client::ae2_world::Machine,
    ) -> Result<(), String> {
        match owner {
            CraftOwner::Part(part) => self.ae2_store_part_machine(*part, machine),
            CraftOwner::Wireless(ctx) => {
                if !self.ae2_wireless_source_valid(host, conn, ctx) { return Err("wireless crafting terminal moved".into()); }
                let stack = host.hub.guests.iter_mut().find(|g| g.conn == conn)
                    .and_then(|g| toolbox_stack_mut(g, ctx.source)).ok_or("wireless crafting terminal moved")?;
                super::ae2_wireless::store_crafting_grid(stack, &machine.slots)
            }
        }
    }
    fn lan_craft_grid_extract(
        &mut self, host: &mut super::lan::LanHost, conn: u32, owner: &CraftOwner,
        pos: (i32, i32, i32), key: &ae2_cell_core::ItemKey,
        plan: &mut super::ae2_buses::FormationPlan,
    ) -> Result<Option<ItemStack>, String> {
        match owner {
            CraftOwner::Part(_) => self.ae2_grid_extract_staged(pos, key, 1, plan),
            CraftOwner::Wireless(ctx) => self.as_guest(host, conn, |game| {
                let previous = game.ae2_wireless_power.replace(ctx.source);
                let result = game.ae2_grid_extract_staged(pos, key, 1, plan);
                game.ae2_wireless_power = previous;
                result
            }).ok_or_else(|| "missing wireless player".to_string())?,
        }
    }
    fn lan_craft_grid_insert(
        &mut self, host: &mut super::lan::LanHost, conn: u32, owner: &CraftOwner,
        address: super::ae2_global::GridAddress, item: &ItemStack,
        plan: &mut super::ae2_buses::FormationPlan,
    ) -> Result<u64, String> {
        match owner {
            CraftOwner::Part(_) => self.ae2_grid_insert_staged_encoded(address, item, item.count as u64, plan),
            CraftOwner::Wireless(ctx) => self.as_guest(host, conn, |game| {
                let previous = game.ae2_wireless_power.replace(ctx.source);
                let result = game.ae2_grid_insert_staged_encoded(address, item, item.count as u64, plan);
                game.ae2_wireless_power = previous;
                result
            }).ok_or_else(|| "missing wireless player".to_string())?,
        }
    }
    fn lan_load_encoding_state(&self,host:&mut super::lan::LanHost,conn:u32)->bool {
        let Some(part)=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.encoding_part) else{return false;};
        let Some(machine)=self.ae2_part_machine(part).ok().flatten() else{return false;};
        let identity=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.encoding_identity);
        if self.ae2_encoding_identity(part)!=identity || identity.is_none(){return false;}
        let Ok(state)=super::ae2_encoding::EncodingState::load(&machine) else{return false;};
        let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()) else{return false;};
        t.pattern_grid=state.inputs;t.processing_outputs=state.outputs;t.fluid_inputs=state.fluid_inputs;t.fluid_outputs=state.fluid_outputs;
        t.substitutes=state.substitutes;t.fluid_substitutes=state.fluid_substitutes;t.processing=state.processing;t.special_mode=state.special_mode;t.stone_recipe=state.stone_recipe;t.encoding_snapshot=Some(machine);true
    }
    fn lan_store_encoding_state(&mut self,host:&super::lan::LanHost,conn:u32)->Result<(),String> {
        let t=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).ok_or("encoding session closed")?;
        let part=t.encoding_part.ok_or("encoding part missing")?;
        let state=super::ae2_encoding::EncodingState{inputs:t.pattern_grid.clone(),outputs:t.processing_outputs.clone(),fluid_inputs:t.fluid_inputs.clone(),fluid_outputs:t.fluid_outputs.clone(),substitutes:t.substitutes,fluid_substitutes:t.fluid_substitutes,processing:t.processing,special_mode:t.special_mode,stone_recipe:t.stone_recipe.clone()};
        let mut machine=self.ae2_part_machine(part)?.ok_or("encoding part missing")?;
        state.save(&mut machine)?;self.ae2_store_part_machine(part,&machine)
    }
    fn lan_encoding_slot_click(&mut self,host:&mut super::lan::LanHost,conn:u32,part:super::ae2_parts::PartAddress,slot:usize,button:i8,input:horizon_client::menu::Input)->Result<(),String> {
        let Some(gi)=host.hub.guests.iter().position(|g|g.conn==conn) else{return Err("player missing".into())};
        let mut machine=self.ae2_part_machine(part)?.ok_or("encoding part missing")?;
        if slot>=2 || machine.slots.len()!=2{return Err("invalid encoding slot".into());}
        let before_inventory=host.hub.guests[gi].inventory.clone();let before_cursor=host.hub.guests[gi].carried.clone();
        let old=machine.slots[slot].clone();
        let accepts=|s:&ItemStack|if slot==0 {s.item.as_ref()=="ae2:blank_pattern"} else {s.item.as_ref()=="ae2:blank_pattern" || super::ae2_processing::decode_pattern(s).is_some() || super::ae2_processing::ae2_decode_processing_pattern(s).is_some() || super::ae2_processing::ae2_decode_special_pattern(s).is_some()};
        if input==horizon_client::menu::Input::QuickMove {
            if let Some(item)=machine.slots[slot].take(){
                let Some(remaining)=self.as_guest(host,conn,|g|g.give(item)) else{return Err("player unavailable".into())};
                machine.slots[slot]=remaining;
            }
        } else if input==horizon_client::menu::Input::Pickup && matches!(button,0|1) {
            let cursor=host.hub.guests[gi].carried.take();let stored=machine.slots[slot].take();
            match (cursor,stored) {
                (None,Some(mut item))=>{let n=if button==1 {(item.count+1)/2}else{item.count};let mut taken=item.clone();taken.count=n;item.count-=n;host.hub.guests[gi].carried=Some(taken);if item.count>0 {machine.slots[slot]=Some(item);}}
                (Some(mut held),None) if accepts(&held)=>{let n=if button==1 {1}else{held.count.min(if slot==1 {1}else{64})};let mut put=held.clone();put.count=n;held.count-=n;machine.slots[slot]=Some(put);if held.count>0 {host.hub.guests[gi].carried=Some(held);}}
                (Some(mut held),Some(mut item)) if slot==0 && accepts(&held) && same(&held,&item)=>{let n=64u32.saturating_sub(item.count).min(if button==1 {1}else{held.count});held.count-=n;item.count+=n;machine.slots[slot]=Some(item);if held.count>0 {host.hub.guests[gi].carried=Some(held);}}
                (Some(held),Some(item)) if button==0 && accepts(&held) && held.count<=if slot==1 {1}else{64}=>{machine.slots[slot]=Some(held);host.hub.guests[gi].carried=Some(item);}
                (held,item)=>{host.hub.guests[gi].carried=held;machine.slots[slot]=item;}
            }
        }
        if slot==1 && machine.slots[1]!=old {if let Some(pattern)=machine.slots[1].as_ref().and_then(super::ae2_encoding::EncodingState::from_pattern){
            if let Err(e)=pattern.save(&mut machine){host.hub.guests[gi].inventory=before_inventory;host.hub.guests[gi].carried=before_cursor;return Err(e);}
        }}
        if let Err(e)=self.ae2_store_part_machine(part,&machine){host.hub.guests[gi].inventory=before_inventory;host.hub.guests[gi].carried=before_cursor;return Err(e);}
        Ok(())
    }
    fn lan_encoding_quick_insert(&mut self,host:&mut super::lan::LanHost,conn:u32,part:super::ae2_parts::PartAddress,index:usize)->Result<bool,String> {
        let gi=host.hub.guests.iter().position(|g|g.conn==conn).ok_or("player missing")?;
        let Some(mut item)=host.hub.guests[gi].inventory.get(index).cloned().flatten() else{return Ok(false)};
        let slot=if item.item.as_ref()=="ae2:blank_pattern" {0} else if super::ae2_encoding::EncodingState::from_pattern(&item).is_some() {1} else{return Ok(false)};
        let mut machine=self.ae2_part_machine(part)?.ok_or("encoding part missing")?;
        let capacity:u32=if slot==0 {64}else{1};
        let room=match machine.slots[slot].as_ref() {None=>capacity,Some(existing) if slot==0 && same(existing,&item)=>capacity.saturating_sub(existing.count),_=>0};
        let moved=item.count.min(room);if moved==0{return Ok(true)};
        if let Some(existing)=machine.slots[slot].as_mut(){existing.count+=moved;}else{let mut placed=item.clone();placed.count=moved;machine.slots[slot]=Some(placed);}
        item.count-=moved;
        if slot==1 {if let Some(state)=machine.slots[1].as_ref().and_then(super::ae2_encoding::EncodingState::from_pattern){state.save(&mut machine)?;}}
        self.ae2_store_part_machine(part,&machine)?;
        host.hub.guests[gi].inventory[index]=(item.count>0).then_some(item);Ok(true)
    }
    pub(super) fn lan_open_ae2_terminal(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
    ) -> bool {
        self.lan_open_ae2_terminal_inner(host, conn, pos, None, None, None, None)
    }
    pub(super) fn lan_open_ae2_view_terminal(&mut self,host:&mut super::lan::LanHost,conn:u32,pos:(i32,i32,i32),part:super::ae2_parts::PartAddress)->bool {
        if !self.ae2_part_machine(part).ok().flatten().is_some_and(|m|m.kind==horizon_client::ae2_world::MachineKind::Terminal) {return false;}
        self.lan_open_ae2_terminal_inner(host,conn,pos,None,None,None,Some(part))
    }
    pub(super) fn lan_open_ae2_wireless_terminal(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
        context: super::ae2_wireless::WirelessContext,
    ) -> bool {
        self.lan_open_ae2_terminal_inner(host, conn, pos, Some(context), None, None, None)
    }
    pub(super) fn lan_open_ae2_crafting_terminal(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
        part: super::ae2_parts::PartAddress,
    ) -> bool {
        if !self
            .ae2_part_machine(part)
            .ok()
            .flatten()
            .is_some_and(|m| m.kind == horizon_client::ae2_world::MachineKind::CraftingTerminal)
        {
            return false;
        }
        self.lan_open_ae2_terminal_inner(host, conn, pos, None, Some(part), None, Some(part))
    }
    pub(super) fn lan_open_ae2_encoding_terminal(&mut self,host:&mut super::lan::LanHost,conn:u32,pos:(i32,i32,i32),part:super::ae2_parts::PartAddress)->bool {
        let Some(machine)=self.ae2_part_machine(part).ok().flatten() else{return false;};
        if machine.kind!=horizon_client::ae2_world::MachineKind::PatternEncodingTerminal || super::ae2_encoding::EncodingState::load(&machine).is_err(){return false;}
        self.lan_open_ae2_terminal_inner(host,conn,pos,None,None,Some(part),Some(part))
    }
    fn lan_open_ae2_terminal_inner(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
        wireless: Option<super::ae2_wireless::WirelessContext>,
        craft_part: Option<super::ae2_parts::PartAddress>,
        encoding_part: Option<super::ae2_parts::PartAddress>,
        view_part: Option<super::ae2_parts::PartAddress>,
    ) -> bool {
        if wireless.as_ref().map_or_else(
            || !self.ae2_has_terminal(pos),
            |ctx| !self.ae2_wireless_validate(host, conn, ctx),
        ) {
            return false;
        }
        let view_identity=view_part.and_then(|part|self.ae2_terminal_part_identity(part));
        if view_part.is_some() && view_identity.is_none(){return false;}
        let encoding_identity=encoding_part.and_then(|part|self.ae2_encoding_identity(part));
        if encoding_part.is_some() && encoding_identity.is_none(){return false;}
        let chest = if wireless.is_none() && view_part.is_none() && self.ae2_local_chest(pos) {
            let Some(chest) = self.chest_terminal_read(pos) else { return false; };
            Some(chest)
        } else { None };
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
        let toolbox_owner = host.hub.guests.iter().find(|g| g.conn == conn).and_then(toolbox_owner);
        let Some(window) = host.hub.open_window(
            conn,
            "minecraft:generic_9x6",
            horizon_protocol::text::literal(if craft_owner(craft_part, wireless.as_ref()).is_some() {
                "AE2 Crafting Terminal"
            } else {
                "AE2 ME Terminal"
            }),
        ) else {
            return true;
        };
        host.play.entry(conn).or_default().ae2_terminal = Some(GuestTerminal {
            window,
            state: 0,
            pos,
            wireless,
            wireless_connected: true,
            owner_dim: self.dim.id().to_owned(),
            craft_part,
            encoding_part,
            view_part,
            view_identity,
            chest_identity: chest.map(|(identity, _)| identity),
            chest_revisions: std::collections::VecDeque::new(),
            priority_edit: None,
            toolbox_owner,
            toolbox_snapshot: None,
            player_snapshot: Vec::new(),
            view_drag: None,
            craft_drag: None,
            wireless_drag: None,
            toolbox_drag: None,
            encoding_identity,
            encoding_snapshot:None,
            encoding_revisions:std::collections::VecDeque::new(),
            craft_snapshot: Vec::new(),
            page: 0,
            job_index: 0,
            craft_amount: 1,
            craft_amount_edit: false,
            encoding: false,
            pattern_grid: vec![None; 81],
            substitutes: false,
            fluid_substitutes: false,
            processing: false,
            special_mode:0,
            stone_recipe:None,
            processing_outputs: vec![None; 27],
            fluid_mode: false,
            processing_page: 0,
            selected_fluid: 0,
            sort_mode: chest.map_or(0, |(_, settings)| settings.sort),
            sort_descending: chest.is_some_and(|(_, settings)| settings.descending),
            filter_mode: chest.map_or(0, |(_, settings)| settings.view),
            search: String::new(),
            search_edit: false,
            fluid_amount: 1000,
            fluid_inputs: vec![None; 81],
            fluid_outputs: vec![None; 27],
            snapshot: Vec::new(),
            displayed_snapshot: Vec::new(),
        });
        if !self.lan_load_encoding_state(host,conn) && encoding_part.is_some() {self.lan_ae2_terminal_closed(host,conn,None);return false;}
        self.lan_send_ae2_terminal(host, conn);
        true
    }
    pub(super) fn lan_ae2_terminal_search(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        value: &str,
    ) -> bool {
        let Some(t) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.ae2_terminal.as_mut())
        else {
            return false;
        };
        t.search = value.chars().take(128).collect();
        t.page = 0;
        t.search_edit = false;
        self.lan_send_ae2_terminal(host, conn);
        true
    }
    fn lan_send_ae2_terminal(&mut self, host: &mut super::lan::LanHost, conn: u32) {
        if host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|t.priority_edit.is_some()){self.lan_send_priority(host,conn);return;}
        let chest = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.chest_identity.map(|identity| (t.pos, identity)));
        if let Some((pos, identity)) = chest {
            if let Some((_, settings)) = self.chest_terminal_read(pos).filter(|(now, _)| *now == identity) {
                if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                    if (t.sort_mode, t.filter_mode, t.sort_descending) != (settings.sort, settings.view, settings.descending) { t.page = 0; }
                    t.sort_mode = settings.sort; t.filter_mode = settings.view; t.sort_descending = settings.descending;
                }
            }
        }
        let toolbox = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| Some((t.window, t.toolbox_owner?)))
            .and_then(|(window, owner)| {
                let g = host.hub.guests.iter().find(|g| g.conn == conn)?;
                let stack = toolbox_stack(g, owner)?.clone();
                let inventory = horizon_client::ae2_network_tool::Inventory::read(&stack).ok()?;
                Some((window, owner, stack, inventory))
            });
        if let Some((window, owner, stack, inventory)) = toolbox {
            let slots: Vec<_> = inventory.slots.iter()
                .map(|v| horizon_client::item_data::canon_stack(v.as_ref())).collect();
            if let Ok(data) = serde_json::to_vec(&serde_json::json!({
                "window": window, "owner": owner, "slots": slots
            })) {
                host.hub.send_to(conn, &C::CustomPayload(
                    horizon_protocol::packets::configuration::CustomPayload {
                        channel: "ae2:terminal_toolbox".into(), data,
                    },
                ));
            }
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                t.toolbox_snapshot = Some(stack);
            }
        }
        if let Some((window,part,identity))=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|Some((t.window,t.view_part?,t.view_identity?))) {
            if self.ae2_terminal_part_identity(part)==Some(identity) {
                if let Some(machine)=self.ae2_part_machine(part).ok().flatten() {
                    if let Ok(cells)=super::ae2_parts::ae2_terminal_view_cells(&machine) {
                        let cells:Vec<_>=cells.iter().map(|v|horizon_client::item_data::canon_stack(v.as_ref())).collect();
                        if let Ok(data)=serde_json::to_vec(&serde_json::json!({"window":window,"cells":cells})) {
                            host.hub.send_to(conn,&C::CustomPayload(horizon_protocol::packets::configuration::CustomPayload{channel:"ae2:view_cells".into(),data}));
                        }
                    }
                }
            }
        }
        if host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|t.encoding_part.is_some()) {self.lan_load_encoding_state(host,conn);}
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let Some((
            pos,
            page,
            craft_amount,
            craft_amount_edit,
            encoding,
            pattern_grid,
            substitutes,
            fluid_substitutes,
            processing,
            special_mode,
            stone_recipe,
            processing_outputs,
            fluid_mode,
            processing_page,
            selected_fluid,
            sort_mode,
            sort_descending,
            filter_mode,
            search,
            search_edit,
            fluid_amount,
            fluid_inputs,
            fluid_outputs,
        )) = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .map(|t| {
                (
                    t.pos,
                    t.page,
                    t.craft_amount,
                    t.craft_amount_edit,
                    t.encoding,
                    t.pattern_grid.clone(),
                    t.substitutes,
                    t.fluid_substitutes,
                    t.processing,
                    t.special_mode,
                    t.stone_recipe.clone(),
                    t.processing_outputs.clone(),
                    t.fluid_mode,
                    t.processing_page,
                    t.selected_fluid,
                    t.sort_mode,
                    t.sort_descending,
                    t.filter_mode,
                    t.search.clone(),
                    t.search_edit,
                    t.fluid_amount,
                    t.fluid_inputs.clone(),
                    t.fluid_outputs.clone(),
                )
            })
        else {
            return;
        };
        let wireless = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.wireless.clone());
        if wireless.as_ref().map_or_else(
            || !self.ae2_has_terminal(pos),
            |ctx| !self.ae2_wireless_source_valid(host, conn, ctx),
        ) {
            let window = host
                .play
                .get(&conn)
                .and_then(|p| p.ae2_terminal.as_ref())
                .map(|t| t.window);
            self.lan_ae2_terminal_closed(host, conn, None);
            if let Some(container) = window {
                host.hub
                    .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
            }
            return;
        }
        // An offline network has no available entries; the next click will
        // still be checked against the authoritative grid before any transfer.
        let connected = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .is_none_or(|t| t.wireless.is_none() || t.wireless_connected);
        let snapshot = if connected {
            self.ae2_terminal_entries(pos).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut craftable = self.ae2_craftable_outputs(pos);
        craftable.extend(self.ae2_craftable_fluids(pos).iter().map(fluid_marker));
        let view_part=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.view_part);
        let view_entries=self.ae2_view_filtered(&snapshot,view_part);
        let displayed = terminal_filter_entries(
            &view_entries,
            &craftable,
            sort_mode,
            sort_descending,
            filter_mode,
            &search,
            &self.pack.names,
        );
        let craft_part = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.craft_part);
        let owner = craft_owner(craft_part, wireless.as_ref());
        let page_size = if owner.is_some() { 27 } else { PAGE_ITEMS };
        let mut items: Vec<_> = (0..54)
            .map(|i| self.ae2_terminal_display(i, page, &displayed, page_size))
            .collect();
        let craft_grid = owner.as_ref()
            .and_then(|owner| self.lan_craft_matrix_read(host, conn, owner).ok())
            .map(|m| m.slots)
            .unwrap_or_default();
        if owner.is_some() && craft_grid.len() == 9 {
            items[27..36].clone_from_slice(&craft_grid);
            items[36] = self.res.recipes().find(&craft_grid, 3).map(|(_, out)| out);
            items[37] = Some(navigation("Clear crafting grid to network"));
            items[38] = Some(navigation("Clear crafting grid to inventory"));
        }
        items[46] = Some(navigation(&format!(
            "Sort: {}",
            match sort_mode { 1 => "amount", 2 => "mod", _ => "name" }
        )));
        items[51] = Some(navigation(if sort_descending {
            "Sort direction: descending"
        } else {
            "Sort direction: ascending"
        }));
        items[47] = Some(navigation(&format!(
            "Filter: {}",
            match filter_mode {
                1 => "stored",
                2 => "craftable",
                _ => "all",
            }
        )));
        items[50] = Some(navigation(&format!(
            "Craft amount: {craft_amount}; click to edit"
        )));
        if wireless.is_some() {
            items[52] = Some(navigation(if connected {
                "Wireless connected"
            } else {
                "Wireless disconnected"
            }));
        }
        if host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|t.chest_identity.is_some()) {
            items[52]=Some(navigation("Priority: click to edit"));
        }
        let job_index = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .map_or(0, |t| t.job_index);
        items[49] = if connected {
            self.ae2_job_button(pos, job_index)
        } else {
            None
        };
        let encoding_part=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.encoding_part);
        if craft_part.is_none() && encoding_part.is_some() {
            items[48] = Some(navigation(if encoding {
                "Back to storage"
            } else {
                "Encode crafting pattern"
            }));
            if encoding {
                items[..48].fill(None);
                if let Some(machine)=encoding_part.and_then(|part|self.ae2_part_machine(part).ok().flatten()) {
                    items[49]=machine.slots.first().cloned().flatten();items[50]=machine.slots.get(1).cloned().flatten();
                }
                let pattern_page = if processing {
                    processing_page.min(8)
                } else {
                    0
                };
                let input_offset = pattern_page * 9;
                let output_offset = pattern_page * 3;
                items[..9].clone_from_slice(&pattern_grid[input_offset..input_offset + 9]);
                items[46] = Some(navigation(match (processing,special_mode) {(true,_)=>"Mode: processing",(_,1)=>"Mode: smithing",(_,2)=>"Mode: stonecutting",_=>"Mode: crafting"}));
                if processing {
                    items[45] = Some(navigation(if fluid_mode {
                        "Ghost type: fluid bucket"
                    } else {
                        "Ghost type: item"
                    }));
                    for digit in 0..10 {
                        items[30 + digit] =
                            Some(navigation(&format!("Fluid amount digit {digit}")));
                    }
                    items[40] = Some(navigation("Clear fluid amount"));
                    items[41] = Some(navigation(&format!("Fluid amount: {fluid_amount} mB")));
                    items[42] = Some(navigation("Previous processing page"));
                    items[43] = Some(navigation(&format!(
                        "Processing page {}/9; click for next",
                        pattern_page + 1
                    )));
                    let available_fluids = self.ae2_grid_fluids(pos).unwrap_or_default();
                    let selected =
                        available_fluids.get(selected_fluid % available_fluids.len().max(1));
                    items[44] = Some(navigation(&selected.map_or_else(
                        || "No network fluid selected".into(),
                        |(k, _)| {
                            format!(
                                "Selected fluid: {} ({} component bytes); click to cycle",
                                k.item_id,
                                k.components.len()
                            )
                        },
                    )));
                    items[18..21]
                        .clone_from_slice(&processing_outputs[output_offset..output_offset + 3]);
                    for (i, f) in fluid_inputs[input_offset..input_offset + 9]
                        .iter()
                        .enumerate()
                    {
                        if let Some(f) = f {
                            let id = super::ae2_fluids::fluid_bucket(&f.key.item_id)
                                .unwrap_or("minecraft:bucket");
                            let mut icon = ItemStack::new(id, 1);
                            icon.data_mut().custom_name =
                                Some(format!("{} mB {}", f.amount, f.key.item_id));
                            items[i] = Some(icon);
                        }
                    }
                    for (i, f) in fluid_outputs[output_offset..output_offset + 3]
                        .iter()
                        .enumerate()
                    {
                        if let Some(f) = f {
                            let id = super::ae2_fluids::fluid_bucket(&f.key.item_id)
                                .unwrap_or("minecraft:bucket");
                            let mut icon = ItemStack::new(id, 1);
                            icon.data_mut().custom_name =
                                Some(format!("{} mB {}", f.amount, f.key.item_id));
                            items[18 + i] = Some(icon);
                        }
                    }
                    if (pattern_grid.iter().any(Option::is_some)
                        || fluid_inputs.iter().any(Option::is_some))
                        && (fluid_outputs.iter().any(Option::is_some)
                            || processing_outputs.iter().any(Option::is_some))
                    {
                        let mut preview = ItemStack::new("ae2:processing_pattern", 1);
                        preview.data_mut().custom_name =
                            Some("Encode item processing pattern".into());
                        items[13] = Some(preview);
                    }
                } else if special_mode!=0 {
                    items[47]=Some(navigation(if substitutes {"Substitutions: on"}else{"Substitutions: off"}));
                    let mut special_grid=vec![None;9];let count=if special_mode==1 {3}else{1};special_grid[..count].clone_from_slice(&pattern_grid[..count]);items[..9].fill(None);items[..count].clone_from_slice(&special_grid[..count]);
                    if special_mode==1 {
                        if let Some((_,result))=self.res.stations().recipes.smithing_recipe_result(&self.res.stations().enchantments,special_grid[0].as_ref(),special_grid[1].as_ref(),special_grid[2].as_ref()){items[13]=Some(result);}
                    } else if let Some(input)=special_grid[0].as_ref() {
                        let recipes=self.res.stations().recipes.stonecutter(input);
                        items[42]=Some(navigation("Previous stonecutting recipe"));items[43]=Some(navigation("Next stonecutting recipe"));
                        if let Some(recipe)=stone_recipe.as_ref().and_then(|id|recipes.iter().find(|r|&r.id==id)) {items[13]=Some(recipe.result.clone());}
                    }
                } else {
                    items[44] = Some(navigation(if fluid_substitutes {
                        "Fluid substitutions: on"
                    } else {
                        "Fluid substitutions: off"
                    }));
                    items[47] = Some(navigation(if substitutes {
                        "Substitutions: on"
                    } else {
                        "Substitutions: off"
                    }));
                    if let Some((_, result)) = self.res.recipes().find(&pattern_grid[..9], 3) {
                        let mut preview = result;
                        preview.data_mut().custom_name = Some("Click to encode pattern".into());
                        items[13] = Some(preview);
                    }
                }
            }
        }
        if craft_amount_edit && !encoding {
            items[..49].fill(None);
            for digit in 0..10 {
                items[digit] = Some(navigation(&format!("Craft amount digit {digit}")));
            }
            items[10] = Some(navigation("Clear craft amount"));
            items[11] = Some(navigation(&format!("Apply craft amount: {craft_amount}")));
        }
        if search_edit && !encoding {
            items[..49].fill(None);
            for (i, ch) in "abcdefghijklmnopqrstuvwxyz0123456789:_-./"
                .chars()
                .enumerate()
            {
                items[i] = Some(navigation(&format!("Search character: {ch}")));
            }
            items[41] = Some(navigation("Search: backspace"));
            items[42] = Some(navigation("Search: clear"));
            items[43] = Some(navigation(&format!("Apply search: {search}")));
        }
        let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let player_snapshot = g.inventory.clone();
        let carried = g.carried.clone();
        let Some(t) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.ae2_terminal.as_mut())
        else {
            return;
        };
        t.craft_snapshot = craft_grid;
        t.player_snapshot = player_snapshot;
        t.snapshot = snapshot;
        t.displayed_snapshot = displayed;
        t.state = (t.state + 1) & 0x7fff;
        if t.chest_identity.is_some() {
            t.chest_revisions.push_back((t.state,ChestTerminalSettings{sort:t.sort_mode,view:t.filter_mode,descending:t.sort_descending}));
            while t.chest_revisions.len()>64 {t.chest_revisions.pop_front();}
        }
        if t.encoding_part.is_some() {
            t.encoding_revisions.push_back((t.state,t.encoding_snapshot.clone()));
            while t.encoding_revisions.len()>64 {t.encoding_revisions.pop_front();}
        }
        let (window, state) = (t.window, t.state);
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container: window,
            state,
            items: items.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        host.hub.send_to(conn, &packet);
    }
    /// Refresh only when the backing inventory changes, or close a removed part.
    pub(super) fn lan_refresh_ae2_terminals(&mut self, host: &mut super::lan::LanHost) {
        let viewers: Vec<_> = host
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
                p.ae2_terminal.as_ref().map(|t| {
                    (
                        conn,
                        t.pos,
                        t.snapshot.clone(),
                        t.wireless.clone(),
                        t.wireless_connected,
                        t.owner_dim.clone(),
                        t.craft_part,
                        t.craft_snapshot.clone(),
                        t.encoding_part,
                        t.encoding_identity,
                        t.encoding_snapshot.clone(),
                        t.view_part,
                        t.view_identity,
                        t.chest_identity,
                        ChestTerminalSettings { sort: t.sort_mode, view: t.filter_mode, descending: t.sort_descending },
                        t.toolbox_owner,
                        t.toolbox_snapshot.clone(),
                        t.player_snapshot.clone(),
                    )
                })
            })
            .collect();
        for (
            conn,
            mut pos,
            previous,
            mut wireless,
            was_connected,
            owner_dim,
            craft_part,
            craft_snapshot,
            encoding_part,
            encoding_identity,
            encoding_snapshot,
            view_part,
            view_identity,
            chest_identity,
            chest_settings,
            toolbox_owner,
            toolbox_snapshot,
            player_snapshot,
        ) in viewers
        {
            let source_valid = (wireless.is_some() || owner_dim == self.dim.id())
                && toolbox_owner.is_none_or(|owner| host.hub.guests.iter()
                    .find(|g| g.conn == conn)
                    .and_then(|g| toolbox_stack(g, owner))
                    .is_some_and(|s| horizon_client::ae2_network_tool::Inventory::read(s).is_ok()))
                && view_part.is_none_or(|part|view_identity.is_some() && self.ae2_terminal_part_identity(part)==view_identity)
                && chest_identity.is_none_or(|identity|self.chest_terminal_read(pos).is_some_and(|(current,_)|current==identity))
                && encoding_part.is_none_or(|part|encoding_identity.is_some() && self.ae2_encoding_identity(part)==encoding_identity && self.ae2_part_machine(part).ok().flatten().is_some_and(|m|m.kind==horizon_client::ae2_world::MachineKind::PatternEncodingTerminal && super::ae2_encoding::EncodingState::load(&m).is_ok()))
                && craft_part.is_none_or(|part| {
                    self.ae2_part_machine(part).ok().flatten().is_some_and(|m| {
                        m.kind == horizon_client::ae2_world::MachineKind::CraftingTerminal
                    })
                })
                && wireless.as_ref().map_or_else(
                    || {
                        host.hub
                            .guests
                            .iter()
                            .find(|g| g.conn == conn)
                            .is_some_and(|g| {
                                !g.spectator() && g.within_block_range([pos.0, pos.1, pos.2], 4.0)
                            })
                    },
                    |ctx| self.ae2_wireless_source_valid(host, conn, ctx),
                );
            let connected = source_valid
                && wireless
                    .as_mut()
                    .is_none_or(|ctx| self.ae2_wireless_tick(host, conn, ctx));
            let old_pos = pos;
            if connected {
                if let Some((nearest, _)) = wireless
                    .as_ref()
                    .and_then(|ctx| self.ae2_wireless_nearest(host, conn, ctx))
                {
                    pos = nearest;
                }
            }
            if let Some(t) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut())
            {
                t.wireless = wireless.clone();
                t.wireless_connected = connected;
                t.pos = pos;
            }
            if !source_valid {
                let window = host
                    .play
                    .get(&conn)
                    .and_then(|p| p.ae2_terminal.as_ref())
                    .map(|t| t.window);
                self.lan_ae2_terminal_closed(host, conn, None);
                if let Some(container) = window {
                    host.hub.send_to(
                        conn,
                        &horizon_protocol::packets::play::Clientbound::ContainerClose(
                            horizon_protocol::packets::play::ContainerClose { container },
                        ),
                    );
                }
                continue;
            }
            if (wireless.is_none() && !self.ae2_has_terminal(pos))
                || toolbox_owner.is_some_and(|owner| host.hub.guests.iter()
                    .find(|g| g.conn == conn)
                    .and_then(|g| toolbox_stack(g, owner)) != toolbox_snapshot.as_ref())
                || host.hub.guests.iter().find(|g| g.conn == conn)
                    .is_some_and(|g| g.inventory != player_snapshot)
                || connected != was_connected
                || pos != old_pos
                || chest_identity.is_some_and(|identity|self.chest_terminal_read(pos).is_some_and(|(current,settings)|current==identity && settings!=chest_settings))
                || craft_part.is_some_and(|part| {
                    self.ae2_part_machine(part)
                        .ok()
                        .flatten()
                        .is_none_or(|m| m.slots != craft_snapshot)
                })
                || craft_owner(None, wireless.as_ref()).is_some_and(|owner|
                    self.lan_craft_matrix_read(host, conn, &owner)
                        .map_or(true, |m| m.slots != craft_snapshot))
                || encoding_part.is_some_and(|part|self.ae2_part_machine(part).ok().flatten()!=encoding_snapshot)
                || (connected
                    && self
                        .ae2_terminal_entries(pos)
                        .map_or(!previous.is_empty(), |now| now != previous))
            {
                self.lan_send_ae2_terminal(host, conn);
            }
        }
    }
    /// A pending drag is a proposal only. Replay across the actual toolbox,
    /// player inventory and optional matrix after checking all their owners.
    fn lan_terminal_toolbox_drag(
        &mut self, host: &mut super::lan::LanHost, conn: u32,
        owner: usize, part: Option<super::ae2_parts::PartAddress>,
        wireless: Option<&super::ae2_wireless::WirelessContext>, slot: i16, button: i8,
    ) {
        use horizon_client::{ae2_network_tool::Inventory, menu::{Kind, Menu}};
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else { return; };
        let button = i32::from(button);
        let mode = button >> 2;
        let stage = button & 3;
        let old = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut())
            .and_then(|t| t.toolbox_drag.take());
        let matrix_owner = craft_owner(part, wireless);
        let machine = matrix_owner.as_ref().and_then(|o| self.lan_craft_matrix_read(host, conn, o).ok());
        let identity = part.and_then(|p| self.ae2_terminal_part_identity(p));
        if stage == 0 {
            let g = &host.hub.guests[gi];
            let tool = toolbox_stack(g, owner).cloned();
            let matrix = if matrix_owner.is_some() {
                machine.as_ref().filter(|m| m.kind == horizon_client::ae2_world::MachineKind::CraftingTerminal && m.slots.len() == 9).map(|m| m.slots.clone())
            } else { Some(vec![None; 9]) };
            let next = match (old, tool, g.carried.clone(), matrix) {
                (None, Some(tool), Some(carried), Some(matrix))
                    if slot == -999 && (0..=2).contains(&mode) && (mode != 2 || g.creative())
                        && (part.is_none() || identity.is_some()) && Inventory::read(&tool).is_ok() =>
                    Some(ToolboxDrag { owner, tool, identity, mode, creative: g.creative(), carried,
                        wireless: wireless.map(|ctx| (ctx.source, ctx.source_owner.clone())),
                        inventory: g.inventory.clone(), armor: g.armor.clone(), offhand: g.offhand.clone(),
                        matrix, slots: Vec::new() }),
                _ => None,
            };
            let started = next.is_some();
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) { t.toolbox_drag = next; }
            if !started { self.lan_send_ae2_terminal(host, conn); }
            return;
        }
        let Some(mut drag) = old else { self.lan_send_ae2_terminal(host, conn); return; };
        let g = &host.hub.guests[gi];
        let valid = drag.owner == owner && drag.mode == mode && drag.creative == g.creative()
            && g.carried.as_ref().is_some_and(|s| s == &drag.carried && Arc::ptr_eq(&s.item, &drag.carried.item))
            && match (wireless, &drag.wireless) {
                (None, None) => g.inventory == drag.inventory && g.offhand == drag.offhand,
                (Some(ctx), Some((source, identity))) => *source == ctx.source && identity.same_object(&ctx.source_owner)
                    && self.ae2_wireless_source_valid(host, conn, ctx)
                    && wireless_unchanged_inventory(ctx.source, &drag.inventory, &g.inventory, &drag.offhand, &g.offhand),
                _ => false,
            }
            && g.armor == drag.armor
            && toolbox_stack(g, owner).is_some_and(|s| s == &drag.tool && Arc::ptr_eq(&s.item, &drag.tool.item))
            && part.is_none_or(|_| identity == drag.identity && identity.is_some())
            && (matrix_owner.is_none() || machine.as_ref().is_some_and(|m| m.slots == drag.matrix));
        if !valid { self.lan_send_ae2_terminal(host, conn); return; }
        if stage == 1 {
            let index = toolbox_drag_slot(slot, matrix_owner.is_some(), owner, wireless.map(|ctx| ctx.source));
            if let Some(index) = index {
                if !drag.slots.contains(&index) { drag.slots.push(index); }
            } else if !(0..=53).contains(&slot) && !(54..=89).contains(&slot) {
                self.lan_send_ae2_terminal(host, conn); return;
            }
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) { t.toolbox_drag = Some(drag); }
            return;
        }
        if stage != 2 || slot != -999 { self.lan_send_ae2_terminal(host, conn); return; }
        let Ok(pocket) = Inventory::read(&drag.tool) else { self.lan_send_ae2_terminal(host, conn); return; };
        if !toolbox_staging_safe(&pocket, |s| self.res.items().get(&s.item).map_or(64, |i| i.max_stack))
            || drag.matrix.iter().flatten().any(|s|
                s.count == 0 || s.count > self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1).min(99))
        { self.lan_send_ae2_terminal(host, conn); return; }
        let before = self.ae2_snapshot_blocks();
        let random = self.world.random.clone();
        let inventory = g.inventory.clone(); let armor = g.armor.clone();
        let offhand = g.offhand.clone(); let carried = g.carried.clone();
        let mut menu = Menu::new(Kind::Chest(2));
        menu.network_tool_slot = Some(owner);
        menu.network_tool_main_limit = Some(9);
        let (result, effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env| {
            for (i, stack) in pocket.slots.iter().chain(drag.matrix.iter()).cloned().enumerate() { menu.set_slot(env, i, stack); }
            let mask = drag.mode << 2;
            menu.click(env, -999, mask, Input::QuickCraft)?;
            for &i in &drag.slots { menu.click(env, i, mask | 1, Input::QuickCraft)?; }
            menu.click(env, -999, mask | 2, Input::QuickCraft)?;
            Ok::<_, &'static str>((0..18).map(|i| menu.slot(i).cloned()).collect::<Vec<_>>())
        });
        let result = result.map_err(str::to_owned).and_then(|next| {
            if toolbox_stack(&host.hub.guests[gi], owner) != Some(&drag.tool) { return Err("network tool moved".into()); }
            if wireless.is_some_and(|ctx| !self.ae2_wireless_source_valid(host, conn, ctx)) { return Err("wireless terminal moved".into()); }
            let mut tool = drag.tool.clone();
            Inventory { slots: std::array::from_fn(|i| next[i].clone()) }.store(&mut tool)?;
            if let (Some(matrix_owner), Some(mut machine)) = (matrix_owner.as_ref(), machine) {
                if machine.slots != next[9..18] {
                    machine.slots = next[9..18].to_vec();
                    self.lan_craft_matrix_store(host, conn, matrix_owner, &machine)?;
                }
            }
            *toolbox_stack_mut(&mut host.hub.guests[gi], owner).ok_or("network tool moved")? = tool;
            Ok(())
        });
        if let Err(message) = result {
            self.ae2_restore_blocks(before);
            let g = &mut host.hub.guests[gi];
            g.inventory = inventory; g.armor = armor; g.offhand = offhand;
            g.carried = carried; self.world.random = random;
            for stack in pocket.slots.iter().chain(drag.matrix.iter()).chain(g.inventory.iter())
                .chain(g.armor.iter()).chain(std::iter::once(&g.offhand)).chain(std::iter::once(&g.carried)).flatten()
            { horizon_client::ae2_stack_identity::synchronize_tracked(stack); }
            self.as_guest(host, conn, |g| g.system_message(Text::literal(format!("Terminal: {message}"))));
        } else { self.apply_guest_menu_effects(host, gi, effects); }
        if wireless.is_some() { host.hub.send_inventory(gi); }
        if let Some(part) = part {
            let viewers: Vec<_> = host.play.iter().filter_map(|(&id, p)|
                p.ae2_terminal.as_ref().filter(|t| t.craft_part == Some(part)).map(|_| id)).collect();
            for viewer in viewers { self.lan_send_ae2_terminal(host, viewer); }
        } else { self.lan_send_ae2_terminal(host, conn); }
    }

    /// Replay wireless player swaps only after excluding the held source.
    fn lan_wireless_player_swap(
        &mut self, host: &mut super::lan::LanHost, conn: u32,
        ctx: &super::ae2_wireless::WirelessContext, slot: i16, button: i8,
    ) {
        use horizon_client::menu::{Kind, Menu};
        let target = match slot {
            54..=80 => Some((slot - 54 + 9) as usize),
            81..=89 => Some((slot - 81) as usize),
            _ => None,
        };
        let source = match button { 0..=8 => Some(button as usize), 40 => Some(40), _ => None };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else { return; };
        if let (Some(target), Some(source)) = (target, source) {
            let g = &host.hub.guests[gi];
            let safe = target != ctx.source && source != ctx.source
                && [toolbox_stack(g, target), toolbox_stack(g, source)].into_iter().flatten()
                    .all(|s| s.count > 0 && s.count <= self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1));
            if safe {
                let inventory = g.inventory.clone(); let armor = g.armor.clone();
                let offhand = g.offhand.clone(); let carried = g.carried.clone();
                let random = self.world.random.clone();
                let mut menu = Menu::new(Kind::Chest(6));
                let (result, effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env|
                    menu.click(env, i32::from(slot), i32::from(button), Input::Swap));
                if result.is_ok() && self.ae2_wireless_source_valid(host, conn, ctx) {
                    self.apply_guest_menu_effects(host, gi, effects);
                } else {
                    let g = &mut host.hub.guests[gi];
                    g.inventory = inventory; g.armor = armor; g.offhand = offhand; g.carried = carried;
                    self.world.random = random;
                    for stack in g.inventory.iter().chain(g.armor.iter()).chain(std::iter::once(&g.offhand))
                        .chain(std::iter::once(&g.carried)).flatten() {
                        horizon_client::ae2_stack_identity::synchronize_tracked(stack);
                    }
                }
            }
        }
        host.hub.send_inventory(gi);
        self.lan_send_ae2_terminal(host, conn);
    }

    /// Accumulate an actual gesture, then commit player and optional item grid
    /// together. A changing battery does not invalidate an otherwise stable
    /// owner; the current battery is retained when the grid is written.
    fn lan_wireless_drag(
        &mut self, host: &mut super::lan::LanHost, conn: u32,
        ctx: &super::ae2_wireless::WirelessContext, slot: i16, button: i8,
    ) {
        use horizon_client::menu::{Kind, Menu};
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else { return; };
        let mode = i32::from(button) >> 2; let stage = i32::from(button) & 3;
        let old = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut())
            .and_then(|t| t.wireless_drag.take());
        let owner = craft_owner(None, Some(ctx));
        let machine = owner.as_ref().and_then(|owner| self.lan_craft_matrix_read(host, conn, owner).ok());
        let matrix = machine.as_ref().map(|m| m.slots.clone());
        if stage == 0 {
            let g = &host.hub.guests[gi];
            let next = match (old, g.carried.clone()) {
                (None, Some(carried)) if slot == -999 && (0..=2).contains(&mode)
                    && (mode != 2 || g.creative()) && (owner.is_none() || matrix.as_ref().is_some_and(|v| v.len() == 9)) =>
                    Some(WirelessDrag { source: ctx.source, identity: ctx.source_owner.clone(), mode,
                        creative: g.creative(), carried, inventory: g.inventory.clone(), armor: g.armor.clone(),
                        offhand: g.offhand.clone(), matrix, slots: Vec::new() }),
                _ => None,
            };
            let started = next.is_some();
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) { t.wireless_drag = next; }
            if !started { self.lan_send_ae2_terminal(host, conn); }
            return;
        }
        let Some(mut drag) = old else { self.lan_send_ae2_terminal(host, conn); return; };
        let g = &host.hub.guests[gi];
        let valid = drag.source == ctx.source && drag.identity.same_object(&ctx.source_owner)
            && self.ae2_wireless_source_valid(host, conn, ctx)
            && drag.mode == mode && drag.creative == g.creative()
            && g.carried.as_ref().is_some_and(|s| s == &drag.carried && Arc::ptr_eq(&s.item, &drag.carried.item))
            && wireless_unchanged_inventory(ctx.source, &drag.inventory, &g.inventory, &drag.offhand, &g.offhand)
            && g.armor == drag.armor && matrix == drag.matrix;
        if !valid { self.lan_send_ae2_terminal(host, conn); return; }
        if stage == 1 {
            let index = if owner.is_some() { craft_drag_menu_slot(slot) } else {
                match slot { 54..=89 => Some(i32::from(slot)), _ => None }
            };
            if let Some(index) = index { if !drag.slots.contains(&index) { drag.slots.push(index); } }
            else if !(0..=53).contains(&slot) { self.lan_send_ae2_terminal(host, conn); return; }
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) { t.wireless_drag = Some(drag); }
            return;
        }
        if stage != 2 || slot != -999 || drag.matrix.iter().flatten().flatten().any(|s|
            s.count == 0 || s.count > self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1).min(99))
        { self.lan_send_ae2_terminal(host, conn); return; }
        let before = self.ae2_snapshot_blocks(); let random = self.world.random.clone();
        let inventory = g.inventory.clone(); let armor = g.armor.clone();
        let offhand = g.offhand.clone(); let carried = g.carried.clone();
        let mut menu = Menu::new(if owner.is_some() { Kind::Crafting } else { Kind::Chest(6) });
        menu.network_tool_slot = Some(ctx.source); menu.network_tool_main_limit = Some(0);
        let (result, effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env| {
            if let Some(matrix) = &drag.matrix {
                for (i, stack) in matrix.iter().cloned().enumerate() { menu.set_slot(env, 1 + i, stack); }
            }
            let mask = drag.mode << 2;
            menu.click(env, -999, mask, Input::QuickCraft)?;
            for &index in &drag.slots { menu.click(env, index, mask | 1, Input::QuickCraft)?; }
            menu.click(env, -999, mask | 2, Input::QuickCraft)?;
            Ok::<_, &'static str>(drag.matrix.as_ref().map(|_| (1..10).map(|i| menu.slot(i).cloned()).collect::<Vec<_>>()))
        });
        let result = result.map_err(str::to_owned).and_then(|next| {
            if !self.ae2_wireless_source_valid(host, conn, ctx) { return Err("wireless terminal moved".into()); }
            if let (Some(owner), Some(mut machine), Some(next)) = (owner.as_ref(), machine, next) {
                machine.slots = next;
                self.lan_craft_matrix_store(host, conn, owner, &machine)?;
            }
            Ok(())
        });
        if result.is_err() {
            self.ae2_restore_blocks(before); self.world.random = random;
            let g = &mut host.hub.guests[gi];
            g.inventory = inventory; g.armor = armor; g.offhand = offhand; g.carried = carried;
            for stack in g.inventory.iter().chain(g.armor.iter()).chain(std::iter::once(&g.offhand))
                .chain(std::iter::once(&g.carried)).flatten() {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
        } else { self.apply_guest_menu_effects(host, gi, effects); }
        host.hub.send_inventory(gi);
        self.lan_send_ae2_terminal(host, conn);
    }

    fn lan_wireless_pickup_all(
        &mut self, host: &mut super::lan::LanHost, conn: u32,
        ctx: &super::ae2_wireless::WirelessContext, slot: i16, button: i8,
    ) -> Result<(), String> {
        use horizon_client::menu::{Kind, Menu};
        if !(0..=1).contains(&button) { return Ok(()); }
        let owner = craft_owner(None, Some(ctx));
        let target = match slot {
            27..=35 if owner.is_some() => (slot - 27) as usize + 36,
            54..=80 => (slot - 54 + 9) as usize,
            81..=89 => (slot - 81) as usize,
            _ => return Ok(()),
        };
        let gi = host.hub.guests.iter().position(|g| g.conn == conn).ok_or("missing guest")?;
        let g = &host.hub.guests[gi];
        let Some(carried) = g.carried.clone() else { return Ok(()); };
        let mut machine = owner.as_ref().map(|owner| self.lan_craft_matrix_read(host, conn, owner)).transpose()?;
        if g.inventory.len() != 36 || machine.as_ref().is_some_and(|m| m.slots.len() != 9) {
            return Err("invalid wireless crafting inventory".into());
        }
        let original: Vec<_> = g.inventory.iter().cloned().chain(machine.iter().flat_map(|m| m.slots.iter().cloned())).collect();
        if original.get(target).is_none_or(|stack| stack.is_some() && target != ctx.source) { return Ok(()); }
        let max = self.res.items().get(&carried.item).map_or(64, |v| v.max_stack).max(1);
        if carried.count >= max { return Ok(()); }
        if !original.iter().enumerate().any(|(i, s)| i != ctx.source && s.as_ref().is_some_and(|s|
            s.same_item_same_components(&carried))) { return Ok(()); }
        if original.iter().enumerate().any(|(i, s)| i != ctx.source && s.as_ref().is_some_and(|s|
            s.same_item_same_components(&carried) && (s.count == 0 || s.count > max))) {
            return Err("oversized pickup-all source".into());
        }
        let (result, next, cursor) = {
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Chest(if owner.is_some() { 5 } else { 4 }));
            for (i, stack) in original.iter().enumerate() {
                if i != ctx.source && stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.set_slot(&env, i, stack.clone());
                }
            }
            menu.carried = Some(carried.clone());
            let result = menu.click(&env, target as i32, i32::from(button), Input::PickupAll);
            let next = original.iter().enumerate().map(|(i, stack)| {
                if i != ctx.source && stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.slot(i).cloned()
                } else { stack.clone() }
            }).collect::<Vec<_>>();
            (result, next, menu.carried)
        };
        if result.is_ok() && next == original && cursor.as_ref() == Some(&carried) { return Ok(()); }
        let commit = result.map_err(str::to_owned).and_then(|()| {
            if !self.ae2_wireless_source_valid(host, conn, ctx) { return Err("wireless terminal moved".into()); }
            if let (Some(owner), Some(machine)) = (owner.as_ref(), machine.as_mut()) {
                machine.slots = next[36..45].to_vec();
                self.lan_craft_matrix_store(host, conn, owner, machine)?;
            }
            Ok(())
        });
        if let Err(message) = commit {
            for stack in original.iter().flatten().chain(std::iter::once(&carried)) {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
            return Err(message);
        }
        // A grid update changed the item source in place. Preserve that current
        // value rather than overwriting it with the pre-gesture snapshot.
        let mut inventory = next[..36].to_vec();
        if ctx.source < 36 { inventory[ctx.source] = host.hub.guests[gi].inventory[ctx.source].clone(); }
        host.hub.guests[gi].inventory = inventory;
        host.hub.guests[gi].carried = cursor;
        Ok(())
    }

    /// Collect the real slots of one quick-craft gesture, then replay it on a
    /// native menu only if the player and physical face are still unchanged.
    fn lan_crafting_terminal_drag(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        part: super::ae2_parts::PartAddress,
        slot: i16,
        button: i8,
    ) {
        use horizon_client::menu::{Kind, Menu};
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else { return; };
        let button = i32::from(button);
        let mode = button >> 2;
        let stage = button & 3;
        let old = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut())
            .and_then(|t| t.craft_drag.take());
        let machine = self.ae2_part_machine(part).ok().flatten();
        if stage == 0 {
            let guest = &host.hub.guests[gi];
            let identity = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
                .filter(|t| t.view_part == Some(part)).and_then(|t| t.view_identity);
            let next = match (old, machine, guest.carried.clone(), identity) {
                (None, Some(machine), Some(carried), Some(identity))
                    if slot == -999 && (0..=2).contains(&mode)
                        && (mode != 2 || guest.creative()) && machine.slots.len() == 9 =>
                {
                    Some(CraftMatrixDrag {
                        identity, mode, creative: guest.creative(), carried,
                        inventory: guest.inventory.clone(), matrix: machine.slots, slots: Vec::new(),
                    })
                }
                _ => None,
            };
            let started = next.is_some();
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                t.craft_drag = next;
            }
            if !started { self.lan_send_ae2_terminal(host, conn); }
            return;
        }

        let Some(mut drag) = old else {
            self.lan_send_ae2_terminal(host, conn);
            return;
        };
        let guest = &host.hub.guests[gi];
        let valid = drag.mode == mode && guest.creative() == drag.creative
            && guest.carried.as_ref().is_some_and(|s| s == &drag.carried
                && std::sync::Arc::ptr_eq(&s.item, &drag.carried.item))
            && guest.inventory == drag.inventory
            && machine.as_ref().is_some_and(|m| m.slots == drag.matrix)
            && self.ae2_terminal_part_identity(part) == Some(drag.identity);
        if !valid {
            self.lan_send_ae2_terminal(host, conn);
            return;
        }
        if stage == 1 {
            if let Some(index) = craft_drag_menu_slot(slot) {
                if !drag.slots.contains(&index) { drag.slots.push(index); }
            } else if !(0..=53).contains(&slot) {
                self.lan_send_ae2_terminal(host, conn);
                return;
            }
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                t.craft_drag = Some(drag);
            }
            return;
        }
        if stage != 2 || slot != -999 {
            self.lan_send_ae2_terminal(host, conn);
            return;
        }

        let mut machine = machine.expect("validated machine");
        let before = self.ae2_snapshot_blocks();
        let inventory = guest.inventory.clone();
        let armor = guest.armor.clone();
        let offhand = guest.offhand.clone();
        let carried = guest.carried.clone();
        let random = self.world.random.clone();
        let mut menu = Menu::new(Kind::Crafting);
        let (next, effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env| {
            for (i, stack) in drag.matrix.iter().cloned().enumerate() {
                menu.set_slot(env, 1 + i, stack);
            }
            let mask = drag.mode << 2;
            menu.click(env, -999, mask, Input::QuickCraft)?;
            for &index in &drag.slots {
                menu.click(env, index, mask | 1, Input::QuickCraft)?;
            }
            menu.click(env, -999, mask | 2, Input::QuickCraft)?;
            Ok::<_, &'static str>((1..10).map(|i| menu.slot(i).cloned()).collect::<Vec<_>>())
        });
        let result = next.map_err(str::to_owned).and_then(|next| {
            machine.slots = next;
            self.ae2_store_part_machine(part, &machine)
        });
        if let Err(message) = result {
            self.ae2_restore_blocks(before);
            host.hub.guests[gi].inventory = inventory;
            host.hub.guests[gi].armor = armor;
            host.hub.guests[gi].offhand = offhand;
            host.hub.guests[gi].carried = carried;
            self.world.random = random;
            let guest = &host.hub.guests[gi];
            for stack in drag.matrix.iter().chain(guest.inventory.iter()).chain(guest.armor.iter())
                .chain(std::iter::once(&guest.offhand)).chain(std::iter::once(&guest.carried)).flatten()
            {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
            self.as_guest(host, conn, |g| {
                g.system_message(Text::literal(format!("Crafting terminal: {message}")))
            });
        } else {
            self.apply_guest_menu_effects(host, gi, effects);
        }
        let viewers: Vec<_> = host.play.iter().filter_map(|(&id, p)| {
            p.ae2_terminal.as_ref().filter(|t| t.craft_part == Some(part)).map(|_| id)
        }).collect();
        for viewer in viewers { self.lan_send_ae2_terminal(host, viewer); }
    }

    /// The nine toolbox slots are owned by the located player stack. Stage
    /// the native menu operation before writing that stack and its effects.
    fn lan_terminal_toolbox_click(
        &mut self, host: &mut super::lan::LanHost, conn: u32, pos: (i32, i32, i32),
        owner: usize, slot: usize, button: i8, input: Input,
    ) -> Result<(), String> {
        use horizon_client::{ae2_network_tool::Inventory, menu::{Kind, Menu}};
        let gi = host.hub.guests.iter().position(|g| g.conn == conn).ok_or("missing guest")?;
        let tool = toolbox_stack(&host.hub.guests[gi], owner).cloned().ok_or("network tool moved")?;
        let mut pocket = Inventory::read(&tool)?;
        if !toolbox_staging_safe(&pocket, |s| self.res.items().get(&s.item).map_or(64, |i| i.max_stack)) {
            return Err("oversized toolbox source".into());
        }
        if slot >= pocket.slots.len() { return Ok(()); }
        if input == Input::QuickMove {
            if !(0..=1).contains(&button) || !self.ae2_terminal_powered(pos) { return Ok(()); }
            let Some(stack) = pocket.slots[slot].clone() else { return Ok(()); };
            let before = self.ae2_snapshot_blocks();
            let moved = match self.ae2_terminal_insert(pos, &stack, u64::from(stack.count)) {
                Ok(moved) if moved <= u64::from(stack.count) => moved as u32,
                Ok(_) => { self.ae2_restore_blocks(before); return Err("invalid grid insert count".into()); }
                Err(e) => { self.ae2_restore_blocks(before); return Err(e); }
            };
            if moved == 0 { return Ok(()); }
            let mut remaining = stack;
            remaining.count -= moved;
            pocket.slots[slot] = (remaining.count > 0).then_some(remaining);
            let mut updated = tool.clone();
            if let Err(e) = pocket.store(&mut updated) {
                self.ae2_restore_blocks(before);
                return Err(e);
            }
            *toolbox_stack_mut(&mut host.hub.guests[gi], owner).ok_or("network tool moved")? = updated;
            return Ok(());
        }
        let allowed = match input {
            Input::Pickup | Input::Throw => (0..=1).contains(&button),
            Input::Swap => (0..=8).contains(&button) || button == 40,
            Input::Clone => button == 2,
            _ => false,
        };
        if !allowed { return Ok(()); }
        if input == Input::Swap {
            let g = &host.hub.guests[gi];
            let source = if button == 40 { g.offhand.as_ref() } else { g.inventory[button as usize].as_ref() };
            if source.is_some_and(|s| s.count > self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1)) {
                return Ok(());
            }
        }
        let inventory = host.hub.guests[gi].inventory.clone();
        let armor = host.hub.guests[gi].armor.clone();
        let offhand = host.hub.guests[gi].offhand.clone();
        let carried = host.hub.guests[gi].carried.clone();
        let random = self.world.random.clone();
        let mut menu = Menu::new(Kind::Chest(1));
        menu.network_tool_slot = Some(owner);
        let (result, effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env| {
            for (i, stack) in pocket.slots.iter().cloned().enumerate() { menu.set_slot(env, i, stack); }
            menu.click(env, slot as i32, i32::from(button), input)?;
            Ok::<_, &'static str>(std::array::from_fn(|i| menu.slot(i).cloned()))
        });
        let result = result.map_err(str::to_owned).and_then(|slots| {
            if toolbox_stack(&host.hub.guests[gi], owner) != Some(&tool) {
                return Err("network tool moved".into());
            }
            let mut updated = tool.clone();
            Inventory { slots }.store(&mut updated)?;
            *toolbox_stack_mut(&mut host.hub.guests[gi], owner).ok_or("network tool moved")? = updated;
            Ok(())
        });
        if let Err(e) = result {
            let guest = &mut host.hub.guests[gi];
            guest.inventory = inventory; guest.armor = armor; guest.offhand = offhand; guest.carried = carried;
            self.world.random = random;
            for stack in pocket.slots.iter().chain(guest.inventory.iter()).chain(guest.armor.iter())
                .chain(std::iter::once(&guest.offhand)).chain(std::iter::once(&guest.carried)).flatten()
            { horizon_client::ae2_stack_identity::synchronize_tracked(stack); }
            return Err(e);
        }
        self.apply_guest_menu_effects(host, gi, effects);
        if input == Input::Swap && button == 40 { host.hub.send_inventory(gi); }
        Ok(())
    }

    /// Double-click in the source menu visits toolbox, then player inventory
    /// (hotbar before main), then the crafting matrix.
    fn lan_terminal_toolbox_pickup_all(
        &mut self, host: &mut super::lan::LanHost, conn: u32,
        owner: usize, part: Option<super::ae2_parts::PartAddress>,
        wireless: Option<&super::ae2_wireless::WirelessContext>, slot: i16, button: i8,
    ) -> Result<(), String> {
        use horizon_client::{ae2_network_tool::Inventory, menu::{Kind, Menu}};
        if !(0..=1).contains(&button) { return Ok(()); }
        let matrix_owner = craft_owner(part, wireless);
        let target = match slot {
            95..=103 => (slot - 95) as usize,
            54..=80 => 18 + (slot - 54) as usize,
            81..=89 => 9 + (slot - 81) as usize,
            27..=35 if matrix_owner.is_some() => 45 + (slot - 27) as usize,
            _ => return Ok(()),
        };
        let gi = host.hub.guests.iter().position(|g| g.conn == conn).ok_or("missing guest")?;
        let g = &host.hub.guests[gi];
        let tool = toolbox_stack(g, owner).cloned().ok_or("network tool moved")?;
        let pocket = Inventory::read(&tool)?;
        if g.inventory.len() != 36 || !toolbox_staging_safe(&pocket, |s| self.res.items().get(&s.item).map_or(64, |i| i.max_stack)) {
            return Err("invalid toolbox inventory".into());
        }
        if wireless.is_some_and(|ctx| !self.ae2_wireless_source_valid(host, conn, ctx)) { return Err("wireless terminal moved".into()); }
        let Some(carried) = g.carried.clone() else { return Ok(()); };
        let mut machine = matrix_owner.as_ref().map(|o| self.lan_craft_matrix_read(host, conn, o)).transpose()?;
        if machine.as_ref().is_some_and(|m| m.kind != horizon_client::ae2_world::MachineKind::CraftingTerminal || m.slots.len() != 9) {
            return Err("invalid crafting matrix".into());
        }
        let mut original: Vec<_> = pocket.slots.iter().chain(g.inventory.iter()).cloned().collect();
        if let Some(machine) = machine.as_ref() { original.extend(machine.slots.iter().cloned()); }
        let locked = |index: usize| toolbox_pickup_locked(index, owner, wireless.map(|ctx| ctx.source));
        // Both DisabledSlot owners are valid double-click targets but cannot be sources.
        if original.get(target).is_none_or(|v| v.is_some() && !locked(target)) { return Ok(()); }
        let max = self.res.items().get(&carried.item).map_or(64, |v| v.max_stack).max(1);
        if carried.count >= max { return Ok(()); }
        if original.iter().enumerate().any(|(i,s)| !locked(i) && s.as_ref().is_some_and(|s|
            s.same_item_same_components(&carried) && (s.count == 0 || s.count > max))) {
            return Err("oversized pickup-all source".into());
        }
        let inventory_before = g.inventory.clone(); let armor_before = g.armor.clone();
        let offhand_before = g.offhand.clone(); let carried_before = g.carried.clone();
        let before = self.ae2_snapshot_blocks();
        let (result, next, cursor) = {
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Chest(if matrix_owner.is_some() {6} else {5}));
            for (i, stack) in original.iter().enumerate() {
                if !locked(i) && stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.set_slot(&env, i, stack.clone());
                }
            }
            menu.carried = Some(carried.clone());
            let result = menu.click(&env, target as i32, i32::from(button), Input::PickupAll);
            let next = original.iter().enumerate().map(|(i, stack)| {
                if !locked(i) && stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.slot(i).cloned()
                } else { stack.clone() }
            }).collect::<Vec<_>>();
            (result, next, menu.carried)
        };
        if result.is_ok() && next == original && cursor.as_ref() == Some(&carried) { return Ok(()); }
        let result = result.map_err(str::to_owned).and_then(|()| {
            if toolbox_stack(&host.hub.guests[gi], owner) != Some(&tool) { return Err("network tool moved".into()); }
            if wireless.is_some_and(|ctx| !self.ae2_wireless_source_valid(host, conn, ctx)) { return Err("wireless terminal moved".into()); }
            let mut updated_tool = tool.clone();
            Inventory { slots: std::array::from_fn(|i| next[i].clone()) }.store(&mut updated_tool)?;
            if let (Some(matrix_owner), Some(machine)) = (matrix_owner.as_ref(), machine.as_mut()) {
                if machine.slots != next[45..54] {
                    machine.slots = next[45..54].to_vec();
                    self.lan_craft_matrix_store(host, conn, matrix_owner, machine)?;
                }
            }
            let mut inventory = next[9..45].to_vec();
            if let Some(ctx) = wireless.filter(|ctx| ctx.source < 36) {
                inventory[ctx.source] = host.hub.guests[gi].inventory[ctx.source].clone();
            }
            if owner < 36 { inventory[owner] = Some(updated_tool.clone()); }
            host.hub.guests[gi].inventory = inventory;
            if owner >= 36 { *toolbox_stack_mut(&mut host.hub.guests[gi], owner).ok_or("network tool moved")? = updated_tool; }
            host.hub.guests[gi].carried = cursor;
            Ok(())
        });
        if let Err(message) = result {
            self.ae2_restore_blocks(before);
            let g = &mut host.hub.guests[gi];
            g.inventory = inventory_before; g.armor = armor_before; g.offhand = offhand_before; g.carried = carried_before;
            for stack in original.iter().chain(g.inventory.iter()).chain(g.armor.iter())
                .chain(std::iter::once(&g.offhand)).chain(std::iter::once(&g.carried)).flatten() {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
            return Err(message);
        }
        Ok(())
    }

    fn lan_crafting_terminal_pickup_all(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        part: super::ae2_parts::PartAddress,
        slot: i16,
        button: i8,
    ) -> Result<(), String> {
        use horizon_client::menu::{Kind, Menu};
        if !(0..=1).contains(&button) { return Ok(()); }
        let target = match slot {
            27..=35 => usize::from((slot - 27) as u16) + 36,
            54..=80 => usize::from((slot - 54) as u16) + 9,
            81..=89 => usize::from((slot - 81) as u16),
            _ => return Ok(()),
        };
        let gi = host.hub.guests.iter().position(|g| g.conn == conn).ok_or("missing guest")?;
        let guest = &host.hub.guests[gi];
        let Some(carried) = guest.carried.clone() else { return Ok(()); };
        let mut machine = self.ae2_part_machine(part)?.ok_or("missing crafting terminal")?;
        if machine.kind != horizon_client::ae2_world::MachineKind::CraftingTerminal
            || machine.slots.len() != 9 || guest.inventory.len() != 36
        {
            return Err("invalid crafting inventory".into());
        }
        let original: Vec<_> = guest.inventory.iter().chain(machine.slots.iter()).cloned().collect();
        if original[target].is_some() { return Ok(()); }
        let max = self.res.items().get(&carried.item).map_or(64, |v| v.max_stack).max(1);
        if carried.count >= max { return Ok(()); }
        if !original.iter().flatten().any(|s| s.same_item_same_components(&carried)) {
            return Ok(());
        }
        // Menu's chest setter caps a stack. Never truncate a malformed source
        // while staging it; unrelated stacks are not copied into the menu.
        if original.iter().flatten().any(|s| s.same_item_same_components(&carried)
            && (s.count == 0 || s.count > max))
        {
            return Err("oversized pickup-all source".into());
        }

        let before = self.ae2_snapshot_blocks();
        let (result, next, cursor) = {
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Chest(5));
            for (i, stack) in original.iter().enumerate() {
                if stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.set_slot(&env, i, stack.clone());
                }
            }
            menu.carried = Some(carried.clone());
            let result = menu.click(&env, target as i32, i32::from(button), Input::PickupAll);
            let next = original.iter().enumerate().map(|(i, stack)| {
                if stack.as_ref().is_some_and(|s| s.same_item_same_components(&carried)) {
                    menu.slot(i).cloned()
                } else {
                    stack.clone()
                }
            }).collect::<Vec<_>>();
            (result, next, menu.carried)
        };
        if result.is_ok() && next == original && cursor.as_ref() == Some(&carried) {
            return Ok(());
        }
        let result = result.map_err(str::to_owned).and_then(|()| {
            machine.slots = next[36..45].to_vec();
            self.ae2_store_part_machine(part, &machine)
        });
        if let Err(message) = result {
            self.ae2_restore_blocks(before);
            for stack in original.iter().flatten().chain(std::iter::once(&carried)) {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
            return Err(message);
        }
        host.hub.guests[gi].inventory = next[..36].to_vec();
        host.hub.guests[gi].carried = cursor;
        Ok(())
    }

    fn lan_crafting_terminal_click(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        owner: &CraftOwner,
        pos: (i32, i32, i32),
        slot: i16,
        button: i8,
        input: Input,
    ) -> Result<(), String> {
        use horizon_client::menu::{Kind, Menu};
        let allowed = match input {
            Input::Pickup | Input::QuickMove => (0..=1).contains(&button),
            Input::Swap => (27..=35).contains(&slot) && ((0..=8).contains(&button) || button == 40),
            Input::Clone => (27..=35).contains(&slot) && button == 2,
            Input::Throw => (27..=35).contains(&slot) && (0..=1).contains(&button),
            _ => false,
        };
        if !allowed {
            return Ok(());
        }
        let gi = host
            .hub
            .guests
            .iter()
            .position(|g| g.conn == conn)
            .ok_or("missing guest")?;
        if input == Input::Swap {
            let guest = &host.hub.guests[gi];
            let source = if button == 40 { guest.offhand.as_ref() } else { guest.inventory.get(button as usize).and_then(Option::as_ref) };
            // Menu::swap can drop a displaced stack when splitting an invalid
            // oversized source; defer that case rather than emit a world item
            // before the face-owned transaction has committed.
            if source.is_some_and(|s| s.count > self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1)) {
                return Ok(());
            }
        }
        let original = self.lan_craft_matrix_read(host, conn, owner)?;
        if original.kind != horizon_client::ae2_world::MachineKind::CraftingTerminal
            || original.slots.len() != 9
        {
            return Err("invalid crafting matrix".into());
        }
        if matches!(owner, CraftOwner::Wireless(_)) && original.slots.iter().flatten().any(|s|
            s.count == 0 || s.count > self.res.items().get(&s.item).map_or(64, |i| i.max_stack).max(1).min(99))
        {
            return Err("oversized wireless crafting matrix".into());
        }
        let (address, view_part) = match owner {
            CraftOwner::Part(part) => (part.host, Some(*part)),
            CraftOwner::Wireless(_) => (self.ae2_local_address(pos).ok_or("invalid wireless network")?, None),
        };
        if host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref()).is_none() {
            return Err("terminal closed".into());
        }
        let before = self.ae2_snapshot_blocks();
        let inventory = host.hub.guests[gi].inventory.clone();
        let armor = host.hub.guests[gi].armor.clone();
        let offhand = host.hub.guests[gi].offhand.clone();
        let carried = host.hub.guests[gi].carried.clone();
        let matrix = original.slots.clone();
        let random = self.world.random.clone();
        let mut effects = super::lan_crafting::GuestMenuEffects::default();
        let result = (|| -> Result<(), String> {
            let mut machine = original;
            let mut formation = super::ae2_buses::FormationPlan::default();
            if slot == 37 || slot == 38 {
                for i in 0..9 {
                    let Some(mut stack) = machine.slots[i].take() else {
                        continue;
                    };
                    if slot == 37 {
                        let moved = self.lan_craft_grid_insert(host, conn, owner, address, &stack, &mut formation)?;
                        stack.count -= moved as u32;
                    } else {
                        let max = self
                            .res
                            .items()
                            .get(&stack.item)
                            .map_or(64, |v| v.max_stack)
                            .max(1);
                        let inv = &mut host.hub.guests[gi].inventory;
                        // CraftingTermMenu clears into hotbar 8..0, then the
                        // remaining 27 slots, first merging and then filling.
                        let order = (0..9).rev().chain(9..36);
                        for index in order.clone() {
                            let Some(dest) = inv.get_mut(index).and_then(Option::as_mut) else { continue; };
                            if dest.same_item_same_components(&stack) && dest.count < max {
                                let n = stack.count.min(max - dest.count);
                                dest.count += n;
                                stack.count -= n;
                                if stack.count == 0 {
                                    break;
                                }
                            }
                        }
                        if stack.count > 0 {
                            for index in order {
                                let Some(dest) = inv.get_mut(index) else { continue; };
                                if dest.is_none() {
                                    let n = stack.count.min(max);
                                    *dest = Some(ItemStack {
                                        count: n,
                                        ..stack.clone()
                                    });
                                    stack.count -= n;
                                    if stack.count == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if stack.count > 0 {
                        machine.slots[i] = Some(stack);
                    }
                }
                self.lan_craft_matrix_store(host, conn, owner, &machine)?;
                self.ae2_commit_formation(formation)?;
                return Ok(());
            }
            let menu_slot = if slot == 36 {
                0
            } else {
                (slot - 27 + 1) as i32
            };
            let shift_result = slot == 36 && input == Input::QuickMove;
            let initial_result = if shift_result {
                self.res
                    .recipes()
                    .find(&machine.slots, 3)
                    .map(|(_, item)| item)
            } else {
                None
            };
            let max_passes = if let Some(output) = &initial_result {
                let max = self
                    .res
                    .items()
                    .get(&output.item)
                    .map_or(64, |item| item.max_stack)
                    .max(1);
                max / output.count.max(1)
            } else if shift_result {
                0
            } else {
                1
            };
            // CraftingTermSlot CRAFT_SHIFT transfers at most one output stack,
            // checking real player capacity before consuming each recipe.
            for _ in 0..max_passes {
                let previous = machine.slots.clone();
                if let Some(expected) = &initial_result {
                    let Some((_, next_result)) = self.res.recipes().find(&previous, 3) else {
                        break;
                    };
                    if next_result.count != expected.count
                        || !next_result.same_item_same_components(expected)
                    {
                        break;
                    }
                    let max = self
                        .res
                        .items()
                        .get(&expected.item)
                        .map_or(64, |item| item.max_stack)
                        .max(1);
                    let room = host.hub.guests[gi]
                        .inventory
                        .iter()
                        .map(|slot| match slot {
                            None => max,
                            Some(item) if item.same_item_same_components(expected) => {
                                max.saturating_sub(item.count)
                            }
                            _ => 0,
                        })
                        .fold(0u64, |sum, n| sum.saturating_add(u64::from(n)));
                    if room < u64::from(expected.count) {
                        break;
                    }
                }
                let mut menu = Menu::new(Kind::Crafting);
                if let CraftOwner::Wireless(ctx) = owner {
                    if input != Input::Swap { menu.network_tool_slot = Some(ctx.source); }
                }
                let (next, pass_effects) = self.guest_menu_op_deferred(host, gi, &mut menu, |menu, env| {
                    for (i, stack) in previous.iter().cloned().enumerate() {
                        menu.set_slot(env, 1 + i, stack);
                    }
                    if shift_result {
                        let original_cursor = menu.carried.take();
                        let _ = menu.click(env, menu_slot, 0, Input::Pickup);
                        if let Some(mut output) = menu.carried.take() {
                            menu.give(env, &mut output);
                            if output.count > 0 {
                                menu.dropped.push(output);
                            }
                        }
                        menu.carried = original_cursor;
                    } else {
                        let _ = menu.click(env, menu_slot, i32::from(button), input);
                    }
                    (1..10).map(|i| menu.slot(i).cloned()).collect::<Vec<_>>()
                });
                effects.append(pass_effects);
                let changed = next != previous;
                machine.slots = next;
                self.lan_craft_matrix_store(host, conn, owner, &machine)?;
                if slot != 36 || !changed {
                    break;
                }
                for i in 0..9 {
                    if machine.slots[i].is_none() {
                        if let Some(old) = previous[i].as_ref() {
                            if let (Ok(key), Some((dim, pos))) =
                                (ae2_lab::key(old), self.ae2_address_parts(address))
                            {
                                if dim == self.dim {
                                    // A disconnected or unpowered network leaves the
                                    // physical matrix empty; it must not undo an
                                    // already completed craft or its remainders.
                                    if self.ae2_view_permits(&key,view_part,horizon_client::ae2_cell_codec::ViewCellScope::ItemsOnly)
                                        && let Ok(Some(item)) = self.lan_craft_grid_extract(host, conn, owner, pos, &key, &mut formation) {
                                        machine.slots[i] = Some(item);
                                    }
                                    // CraftingTermSlot permits a different component
                                    // or damage variant of the same item only when
                                    // the recipe still produces the identical result.
                                    let damageable = self
                                        .res
                                        .items()
                                        .get(&old.item)
                                        .is_some_and(|v| v.max_damage > 0);
                                    if machine.slots[i].is_none()
                                        && (old.extra.is_some() || damageable)
                                    {
                                        let expected = self
                                            .res
                                            .recipes()
                                            .find(&previous, 3)
                                            .map(|(_, out)| out);
                                        if let (Some(expected), Ok(entries)) =
                                            (expected, self.ae2_grid_inventory(pos))
                                        {
                                            for (candidate, _) in entries {
                                                if !self.ae2_view_permits(&candidate,view_part,horizon_client::ae2_cell_codec::ViewCellScope::ItemsOnly) || candidate.item_id != key.item_id
                                                    || candidate == key
                                                {
                                                    continue;
                                                }
                                                let Ok(sample) = ae2_lab::prototype(&candidate)
                                                else {
                                                    continue;
                                                };
                                                let mut grid = previous.clone();
                                                grid[i] = Some(sample);
                                                let valid =
                                                    self.res.recipes().find(&grid, 3).is_some_and(
                                                        |(_, out)| {
                                                            out.count == expected.count
                                                                && out.same_item_same_components(
                                                                    &expected,
                                                                )
                                                        },
                                                    );
                                                if valid {
                                                    if let Ok(Some(item)) =
                                                        self.lan_craft_grid_extract(host, conn, owner, pos, &candidate, &mut formation)
                                                    {
                                                        machine.slots[i] = Some(item);
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.lan_craft_matrix_store(host, conn, owner, &machine)?;
                if input != Input::QuickMove {
                    break;
                }
            }
            self.ae2_commit_formation(formation)?;
            Ok(())
        })();
        if result.is_err() {
            self.ae2_restore_blocks(before);
            host.hub.guests[gi].inventory = inventory;
            host.hub.guests[gi].armor = armor;
            host.hub.guests[gi].offhand = offhand;
            host.hub.guests[gi].carried = carried;
            self.world.random = random;
            // Menu operations can split tracked live stacks before a storage
            // error. Restore the retained values with the owned slots.
            let guest = &host.hub.guests[gi];
            for stack in matrix.iter().chain(guest.inventory.iter()).chain(guest.armor.iter())
                .chain(std::iter::once(&guest.offhand)).chain(std::iter::once(&guest.carried)).flatten()
            {
                horizon_client::ae2_stack_identity::synchronize_tracked(stack);
            }
        } else {
            self.apply_guest_menu_effects(host, gi, effects);
        }
        result
    }
    pub(super) fn lan_ae2_terminal_click(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        p: &horizon_protocol::packets::play::ContainerClick,
    ) -> bool {
        let Some((pos, page, wireless, connected, craft_amount, craft_amount_edit)) = host
            .play
            .get(&conn)
            .and_then(|x| x.ae2_terminal.as_ref())
            .filter(|t| t.window == p.container)
            .map(|t| {
                (
                    t.pos,
                    t.page,
                    t.wireless.clone(),
                    t.wireless_connected,
                    t.craft_amount,
                    t.craft_amount_edit,
                )
            })
        else {
            return false;
        };
        if let Some((identity, shown)) = host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref())
            .and_then(|t| t.chest_identity.map(|identity| (identity, ChestTerminalSettings { sort: t.sort_mode, view: t.filter_mode, descending: t.sort_descending }))) {
            match self.chest_terminal_read(pos) {
                Some((current, settings)) if current == identity && settings == shown => {}
                Some((current, _)) if current == identity => { self.lan_send_ae2_terminal(host, conn); return true; }
                _ => {
                    self.lan_ae2_terminal_closed(host, conn, None);
                    host.hub.send_to(conn, &C::ContainerClose(play::ContainerClose { container: p.container }));
                    return true;
                }
            }
            if matches!(p.slot, 46 | 47 | 51) && super::lan_crafting::input(p.input) == Input::Pickup
                && host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_none_or(|t|!chest_revision_matches(&t.chest_revisions,p.state,shown)) {
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        if host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|t.priority_edit.is_some()){return self.lan_ae2_priority_click(host,conn,p);}
        let craft_part = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.craft_part);
        let encoding_part=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.encoding_part);
        if let Some(part)=encoding_part {
            let current=self.ae2_part_machine(part).ok().flatten();
            // A broadcast can update the session before an already queued
            // click arrives. Compare the recipe actually displayed at that
            // revision, not merely the latest counter (which also advances on
            // harmless inventory refreshes).
            let stale=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_none_or(|t|
                !encoding_revision_matches(&t.encoding_revisions,p.state,current.as_ref()));
            if stale && (0..=53).contains(&p.slot) {
                if self.lan_load_encoding_state(host,conn){self.lan_send_ae2_terminal(host,conn);return true;}
            }
        }
        if encoding_part.is_some() && !self.lan_load_encoding_state(host,conn) {
            self.lan_ae2_terminal_closed(host,conn,None);
            host.hub.send_to(conn,&horizon_protocol::packets::play::Clientbound::ContainerClose(horizon_protocol::packets::play::ContainerClose{container:p.container}));
            return true;
        }
        let owner = craft_owner(craft_part, wireless.as_ref());
        let page_size = if owner.is_some() { 27 } else { PAGE_ITEMS };
        let valid = wireless.as_ref().map_or_else(
            || {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .is_some_and(|g| {
                        !g.spectator() && g.within_block_range([pos.0, pos.1, pos.2], 4.0)
                    })
                    && self.ae2_has_terminal(pos)
                    && host
                        .play
                        .get(&conn)
                        .and_then(|p| p.ae2_terminal.as_ref())
                        .is_some_and(|t| t.owner_dim == self.dim.id())
            },
            |ctx| self.ae2_wireless_source_valid(host, conn, ctx),
        );
        let valid = valid
            && craft_part.is_none_or(|part| {
                self.ae2_part_machine(part).ok().flatten().is_some_and(|m| {
                    m.kind == horizon_client::ae2_world::MachineKind::CraftingTerminal
                })
            })
            && host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|
                t.view_part.is_none_or(|part|t.view_identity.is_some() && self.ae2_terminal_part_identity(part)==t.view_identity));
        if !valid {
            self.lan_ae2_terminal_closed(host, conn, None);
            host.hub.send_to(
                conn,
                &horizon_protocol::packets::play::Clientbound::ContainerClose(
                    horizon_protocol::packets::play::ContainerClose {
                        container: p.container,
                    },
                ),
            );
            return true;
        }
        let toolbox_owner = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
            .and_then(|t| t.toolbox_owner);
        // Apply the wireless source lock before any toolbox or matrix route.
        // In particular a predicted offhand swap needs an inventory update as
        // well as a container update to put the real owner back on the client.
        if let Some(ctx) = &wireless {
            let player_slot = match p.slot {
                54..=80 => Some((p.slot - 54 + 9) as usize),
                81..=89 => Some((p.slot - 81) as usize),
                _ => None,
            };
            let input = super::lan_crafting::input(p.input);
            if (player_slot == Some(ctx.source) && !matches!(input, Input::QuickCraft | Input::PickupAll))
                || (input == Input::Swap && wireless_toolbox_swap_refused(ctx.source, toolbox_owner, p.button))
            {
                if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                    t.wireless_drag = None; t.toolbox_drag = None; t.craft_drag = None;
                }
                if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                    host.hub.send_inventory(gi);
                }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        if let Some(owner) = toolbox_owner {
            let exists = host.hub.guests.iter().find(|g| g.conn == conn)
                .and_then(|g| toolbox_stack(g, owner))
                .is_some_and(|s| horizon_client::ae2_network_tool::Inventory::read(s).is_ok());
            if !exists {
                self.lan_ae2_terminal_closed(host, conn, None);
                host.hub.send_to(conn, &C::ContainerClose(play::ContainerClose { container: p.container }));
                return true;
            }
            let player_slot = match p.slot {
                54..=80 => Some((p.slot - 54 + 9) as usize),
                81..=89 => Some((p.slot - 81) as usize),
                _ => None,
            };
            let input = super::lan_crafting::input(p.input);
            if player_slot == Some(owner) && input == Input::Clone {
                let gi = host.hub.guests.iter().position(|g| g.conn == conn);
                if let Some(gi) = gi {
                    let g = &mut host.hub.guests[gi];
                    if g.creative() && g.carried.is_none() {
                        if let Some(mut copy) = toolbox_stack(g, owner).cloned() {
                            horizon_client::ae2_stack_identity::detach_if_tracked(&mut copy);
                            copy.count = self.res.items().get(&copy.item).map_or(64, |i| i.max_stack).max(1);
                            g.carried = Some(copy);
                        }
                    }
                }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
            if (player_slot == Some(owner) && input != Input::PickupAll)
                || (input == Input::Swap && owner == 40)
            {
                // A rejected drag-add is simply omitted from the pending set.
                if input != Input::QuickCraft {
                    if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                        t.craft_drag = None; t.toolbox_drag = None;
                    }
                    if input == Input::Swap && owner == 40 {
                        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                            host.hub.send_inventory(gi);
                        }
                    }
                    self.lan_send_ae2_terminal(host, conn);
                }
                return true;
            }
            // Vanilla SWAP accesses the hotbar directly, bypassing the
            // disabled slot. If the target accepts the tool, the locator is
            // checked again after the exchange.
            if input == Input::Swap && owner < 9 && usize::try_from(p.button).ok() == Some(owner)
                && player_slot.is_some()
            {
                if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                    let mut menu = horizon_client::menu::Menu::new(horizon_client::menu::Kind::Chest(6));
                    self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                        let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), Input::Swap);
                    });
                }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        let panel_drag=super::lan_crafting::input(p.input)==Input::QuickCraft && (p.slot==-990 || p.slot==-991 || (90..95).contains(&p.slot));
        if super::lan_crafting::input(p.input) != Input::QuickCraft || panel_drag {
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                t.craft_drag = None;
                t.toolbox_drag = None;
                t.wireless_drag = None;
            }
        }
        if panel_drag {
            let Some(gi)=host.hub.guests.iter().position(|g|g.conn==conn) else{return true;};
            let mode=(i32::from(p.button)>>2)&3;
            let stage=i32::from(p.button)&3;
            if p.slot==-990 {
                let carried=host.hub.guests[gi].carried.clone().filter(|s|s.item.as_ref()=="ae2:view_cell" && s.count==1 && host.hub.guests[gi].creative() && mode==2 && stage==0);
                if let Some(t)=host.play.get_mut(&conn).and_then(|v|v.ae2_terminal.as_mut()) {t.view_drag=carried.map(|carried|ViewCellDrag{carried,slots:Vec::new()});}
            } else if (90..95).contains(&p.slot) {
                if let Some(drag)=host.play.get_mut(&conn).and_then(|v|v.ae2_terminal.as_mut()).and_then(|t|t.view_drag.as_mut()) {
                    let i=(p.slot-90) as usize;if mode==2 && stage==1 && host.hub.guests[gi].carried.as_ref()==Some(&drag.carried) && !drag.slots.contains(&i){drag.slots.push(i);}
                }
            } else {
                let session=host.play.get_mut(&conn).and_then(|v|v.ae2_terminal.as_mut());
                let part=session.as_ref().and_then(|t|t.view_part);let drag=session.and_then(|t|t.view_drag.take());
                if let (Some(part),Some(drag))=(part,drag) {
                    if mode==2 && stage==2 && host.hub.guests[gi].creative() && host.hub.guests[gi].carried.as_ref()==Some(&drag.carried) {
                        if let Some(mut machine)=self.ae2_part_machine(part).ok().flatten() {
                            if let Ok(mut cells)=super::ae2_parts::ae2_terminal_view_cells(&machine) {
                                for i in drag.slots {if cells[i].is_none(){cells[i]=Some(drag.carried.clone());}}
                                if super::ae2_parts::ae2_set_terminal_view_cells(&mut machine,&cells).is_ok() && self.ae2_store_part_machine(part,&machine).is_ok() {
                                    let viewers:Vec<_>=host.play.iter().filter_map(|(&c,v)|v.ae2_terminal.as_ref().filter(|t|t.view_part==Some(part)).map(|_|c)).collect();
                                    for viewer in viewers {self.lan_send_ae2_terminal(host,viewer);}return true;
                                }
                            }
                        }
                    }
                }
                self.lan_send_ae2_terminal(host,conn);
            }
            return true;
        }
        if (54..90).contains(&p.slot) && super::lan_crafting::input(p.input)==Input::QuickMove {
            let part=host.play.get(&conn).and_then(|v|v.ae2_terminal.as_ref()).and_then(|t|t.view_part);
            if let (Some(part),Some(gi))=(part,host.hub.guests.iter().position(|g|g.conn==conn)) {
                let slot=if p.slot<81 {(p.slot-54+9) as usize}else{(p.slot-81) as usize};
                if let Some(mut held)=host.hub.guests[gi].inventory[slot].clone().filter(|s|s.item.as_ref()=="ae2:view_cell" && s.count>0) {
                    if let Some(mut machine)=self.ae2_part_machine(part).ok().flatten() {
                        if let Ok(mut cells)=super::ae2_parts::ae2_terminal_view_cells(&machine) {
                            if let Some(i)=cells.iter().position(Option::is_none) {
                                let mut one=held.clone();one.count=1;cells[i]=Some(one);held.count-=1;
                                if super::ae2_parts::ae2_set_terminal_view_cells(&mut machine,&cells).is_ok() && self.ae2_store_part_machine(part,&machine).is_ok() {
                                    host.hub.guests[gi].inventory[slot]=(held.count>0).then_some(held);
                                    let viewers:Vec<_>=host.play.iter().filter_map(|(&c,v)|v.ae2_terminal.as_ref().filter(|t|t.view_part==Some(part)).map(|_|c)).collect();
                                    for viewer in viewers {self.lan_send_ae2_terminal(host,viewer);}return true;
                                }
                            }
                        }
                    }
                    self.lan_send_ae2_terminal(host,conn);return true;
                }
            }
        }
        // Five physical side-panel slots use dedicated packet indices beyond
        // the generic chest/player range; they never alias network entries.
        if (90..95).contains(&p.slot) {
            let part=host.play.get(&conn).and_then(|v|v.ae2_terminal.as_ref()).and_then(|t|t.view_part);
            if let (Some(part),Some(gi))=(part,host.hub.guests.iter().position(|g|g.conn==conn)) {
                if let Some(mut machine)=self.ae2_part_machine(part).ok().flatten() {
                    if let Ok(mut cells)=super::ae2_parts::ae2_terminal_view_cells(&machine) {
                        let i=(p.slot-90) as usize;let input=super::lan_crafting::input(p.input);
                        let mut carried=host.hub.guests[gi].carried.clone();let mut inventory=host.hub.guests[gi].inventory.clone();let mut offhand=host.hub.guests[gi].offhand.clone();let mut changed=false;let mut drops=Vec::new();
                        if input==Input::Pickup && p.button==2 && host.hub.guests[gi].creative() && cells[i].is_none() {
                            if let Some(held)=carried.as_ref().filter(|s|s.item.as_ref()=="ae2:view_cell" && s.count==1) {cells[i]=Some(held.clone());changed=true;}
                        } else if input==Input::Pickup && (0..=1).contains(&p.button) {
                            match (cells[i].clone(),carried.clone()) {
                                (Some(cell),None)=>{cells[i]=None;carried=Some(cell);changed=true;}
                                (None,Some(mut held)) if held.item.as_ref()=="ae2:view_cell" && held.count>0=>{let mut one=held.clone();one.count=1;cells[i]=Some(one);held.count-=1;carried=(held.count>0).then_some(held);changed=true;}
                                (Some(cell),Some(held)) if held.item.as_ref()=="ae2:view_cell" && held.count==1=>{cells[i]=Some(held);carried=Some(cell);changed=true;}
                                _=>{}
                            }
                        } else if input==Input::QuickMove {
                            if let Some(cell)=cells[i].clone() {if let Some(index)=inventory.iter().position(Option::is_none){inventory[index]=Some(cell);cells[i]=None;changed=true;}}
                        } else if input==Input::Swap && (0..=8).contains(&p.button) {
                            let hotbar=p.button as usize;
                            if inventory[hotbar].as_ref().is_none_or(|s|s.item.as_ref()=="ae2:view_cell" && s.count==1) {
                                std::mem::swap(&mut inventory[hotbar],&mut cells[i]);changed=true;
                            }
                        } else if input==Input::Swap && p.button==40 {
                            if offhand.as_ref().is_none_or(|s|s.item.as_ref()=="ae2:view_cell" && s.count==1) {
                                std::mem::swap(&mut offhand,&mut cells[i]);changed=true;
                            }
                        } else if input==Input::Clone && host.hub.guests[gi].creative() && carried.is_none() {
                            if let Some(cell)=cells[i].clone() {carried=Some(cell);changed=true;}
                        } else if input==Input::Throw && (0..=1).contains(&p.button) {
                            if let Some(cell)=cells[i].take() {drops.push(cell);changed=true;}
                        }
                        if changed && super::ae2_parts::ae2_set_terminal_view_cells(&mut machine,&cells).is_ok() && self.ae2_store_part_machine(part,&machine).is_ok() {
                            host.hub.guests[gi].carried=carried;host.hub.guests[gi].inventory=inventory;host.hub.guests[gi].offhand=offhand;
                            if input==Input::Swap && p.button==40 {host.hub.send_inventory(gi);}
                            self.drop_guest_items(host,gi,drops);
                            let viewers:Vec<_>=host.play.iter().filter_map(|(&c,v)|v.ae2_terminal.as_ref().filter(|t|t.view_part==Some(part)).map(|_|c)).collect();
                            for viewer in viewers {self.lan_send_ae2_terminal(host,viewer);}
                            return true;
                        }
                    }
                }
            }
            self.lan_send_ae2_terminal(host,conn);return true;
        }
        let input = super::lan_crafting::input(p.input);
        let has_crafting_matrix = owner.is_some();
        if let Some(owner) = toolbox_owner {
            if input == Input::PickupAll && ((95..=103).contains(&p.slot)
                || (54..=89).contains(&p.slot) || (has_crafting_matrix && (27..=35).contains(&p.slot)))
            {
                if let Err(message) = self.lan_terminal_toolbox_pickup_all(host, conn, owner, craft_part, wireless.as_ref(), p.slot, p.button) {
                    self.as_guest(host, conn, |g| g.system_message(Text::literal(format!("Terminal: {message}"))));
                }
                if wireless.is_some() { if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) { host.hub.send_inventory(gi); } }
                if let Some(part) = craft_part {
                    let viewers: Vec<_> = host.play.iter().filter_map(|(&id, p)|
                        p.ae2_terminal.as_ref().filter(|t| t.craft_part == Some(part)).map(|_| id)).collect();
                    for viewer in viewers { self.lan_send_ae2_terminal(host, viewer); }
                } else { self.lan_send_ae2_terminal(host, conn); }
                return true;
            }
            if (95..=103).contains(&p.slot) && input != Input::QuickCraft {
                if let Err(message) = self.lan_terminal_toolbox_click(host, conn, pos, owner, (p.slot - 95) as usize, p.button, input) {
                    self.as_guest(host, conn, |g| g.system_message(Text::literal(format!("Terminal: {message}"))));
                }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        let connected = connected
            && wireless
                .as_ref()
                .is_none_or(|ctx| self.ae2_wireless_validate(host, conn, ctx));
        if !connected {
            if let Some(t) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut())
            {
                t.wireless_connected = false;
            }
        }
        if !connected
            && ((0..=53).contains(&p.slot) || super::lan_crafting::input(p.input) != Input::Pickup)
        {
            if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                t.craft_drag = None;
                t.wireless_drag = None;
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if let Some(ctx) = &wireless {
            let input = super::lan_crafting::input(p.input);
            let overlay = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
                .is_some_and(|t| t.craft_amount_edit || t.search_edit || t.encoding);
            if input == Input::QuickCraft && toolbox_owner.is_none() {
                if !overlay { self.lan_wireless_drag(host, conn, ctx, p.slot, p.button); }
                else if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) { t.wireless_drag = None; }
                return true;
            }
            if input == Input::PickupAll {
                if !overlay {
                    if let Err(message) = self.lan_wireless_pickup_all(host, conn, ctx, p.slot, p.button) {
                        self.as_guest(host, conn, |g| g.system_message(Text::literal(format!("Wireless terminal: {message}"))));
                    }
                }
                if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) { host.hub.send_inventory(gi); }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
            if input == Input::Swap && !(owner.is_some() && (27..=35).contains(&p.slot) && !overlay) {
                self.lan_wireless_player_swap(host, conn, ctx, p.slot, p.button);
                return true;
            }
        }
        if let Some(owner) = toolbox_owner {
            if super::lan_crafting::input(p.input) == Input::QuickCraft {
                let card_cursor = host.hub.guests.iter().find(|g| g.conn == conn)
                    .and_then(|g| g.carried.as_ref())
                    .is_some_and(|s| horizon_client::ae2_network_tool::is_card(&s.item));
                let overlay = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
                    .is_some_and(|t| t.craft_amount_edit || t.search_edit || (t.encoding && !card_cursor));
                if !overlay {
                    self.lan_terminal_toolbox_drag(host, conn, owner, craft_part, wireless.as_ref(), p.slot, p.button);
                } else if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                    t.toolbox_drag = None;
                }
                return true;
            }
        }
        if let Some(part) = craft_part {
            let overlay = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
                .is_some_and(|t| t.craft_amount_edit || t.search_edit || t.encoding);
            if !overlay && super::lan_crafting::input(p.input) == Input::QuickCraft {
                self.lan_crafting_terminal_drag(host, conn, part, p.slot, p.button);
                return true;
            }
            if overlay {
                if let Some(t) = host.play.get_mut(&conn).and_then(|p| p.ae2_terminal.as_mut()) {
                    t.craft_drag = None;
                }
            }
        }
        let current = if connected {
            self.ae2_terminal_entries(pos)
        } else {
            Ok(Vec::new())
        };
        let craftable_now = {
            let mut keys = self.ae2_craftable_outputs(pos);
            keys.extend(self.ae2_craftable_fluids(pos).iter().map(fluid_marker));
            keys
        };
        let matches_snapshot = host
            .play
            .get(&conn)
            .and_then(|x| x.ae2_terminal.as_ref())
            .is_some_and(|t| {
                current.as_ref().is_ok_and(|v| {
                    *v == t.snapshot
                        && terminal_filter_entries(
                            &self.ae2_view_filtered(v,t.view_part),
                            &craftable_now,
                            t.sort_mode,
                            t.sort_descending,
                            t.filter_mode,
                            &t.search,
                            &self.pack.names,
                        ) == t.displayed_snapshot
                })
            });
        // An indexed storage view can be reordered between updates. Only
        // those virtual-key clicks require the exact displayed revision;
        // physical matrix, player and control slots use current server state.
        let virtual_key_click = host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|
            !t.encoding && !t.craft_amount_edit && !t.search_edit && p.slot>=0 && (p.slot as usize)<page_size);
        if virtual_key_click && (!matches_snapshot || host.play.get(&conn).and_then(|x|x.ae2_terminal.as_ref()).is_none_or(|t|t.state!=p.state)) {
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if let Some(part) = craft_part {
            let overlay = host
                .play
                .get(&conn)
                .and_then(|p| p.ae2_terminal.as_ref())
                .is_some_and(|t| t.craft_amount_edit || t.search_edit || t.encoding);
            if !overlay && super::lan_crafting::input(p.input) == Input::PickupAll
                && ((27..=35).contains(&p.slot) || (54..=89).contains(&p.slot))
            {
                if let Err(message) = self.lan_crafting_terminal_pickup_all(host, conn, part, p.slot, p.button) {
                    self.as_guest(host, conn, |g| {
                        g.system_message(Text::literal(format!("Crafting terminal: {message}")))
                    });
                }
                let viewers: Vec<_> = host.play.iter().filter_map(|(&id, p)| {
                    p.ae2_terminal.as_ref().filter(|t| t.craft_part == Some(part)).map(|_| id)
                }).collect();
                for viewer in viewers { self.lan_send_ae2_terminal(host, viewer); }
                return true;
            }
        }
        if let Some(owner) = owner.as_ref() {
            let overlay = host.play.get(&conn).and_then(|p| p.ae2_terminal.as_ref())
                .is_some_and(|t| t.craft_amount_edit || t.search_edit || t.encoding);
            if !overlay && (27..=38).contains(&p.slot) {
                let result = self.lan_crafting_terminal_click(
                    host,
                    conn,
                    owner,
                    pos,
                    p.slot,
                    p.button,
                    super::lan_crafting::input(p.input),
                );
                if let Err(message) = result {
                    self.as_guest(host, conn, |g| {
                        g.system_message(Text::literal(format!("Crafting terminal: {message}")))
                    });
                }
                if matches!(owner, CraftOwner::Wireless(ctx) if ctx.source == super::shield::OFFHAND_SLOT
                    || (super::lan_crafting::input(p.input) == Input::Swap && p.button == 40)) {
                    if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                        host.hub.send_inventory(gi);
                    }
                }
                let viewers: Vec<_> = match owner {
                    CraftOwner::Wireless(_) => vec![conn],
                    CraftOwner::Part(part) => host
                    .play
                    .iter()
                    .filter_map(|(&id, p)| {
                        p.ae2_terminal
                            .as_ref()
                            .filter(|t| t.craft_part == Some(*part))
                            .map(|_| id)
                    })
                    .collect(),
                };
                for viewer in viewers {
                    self.lan_send_ae2_terminal(host, viewer);
                }
                return true;
            }
        }
        if p.slot == 48
            && craft_part.is_none()
            && encoding_part.is_some()
            && !host
                .play
                .get(&conn)
                .and_then(|p| p.ae2_terminal.as_ref())
                .is_some_and(|t| t.search_edit)
            && super::lan_crafting::input(p.input) == Input::Pickup
        {
            if let Some(t) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut())
            {
                t.encoding = !t.encoding;
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        let encoding = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .is_some_and(|t| t.encoding);
        if encoding && (54..90).contains(&p.slot) && super::lan_crafting::input(p.input)==horizon_client::menu::Input::QuickMove {
            if let Some(part)=encoding_part {
                let index=if p.slot<81 {9+(p.slot as usize-54)} else {p.slot as usize-81};
                match self.lan_encoding_quick_insert(host,conn,part,index) {
                    Ok(true)=>{self.lan_load_encoding_state(host,conn);self.lan_send_ae2_terminal(host,conn);return true;}
                    Err(message)=>{self.as_guest(host,conn,|g|g.system_message(Text::literal(format!("Encoding: {message}"))));self.lan_send_ae2_terminal(host,conn);return true;}
                    Ok(false)=>{}
                }
            }
        }
        if encoding && (p.slot==49 || p.slot==50) {
            if let Some(part)=encoding_part {
                if let Err(message)=self.lan_encoding_slot_click(host,conn,part,(p.slot-49) as usize,p.button,super::lan_crafting::input(p.input)) {
                    self.as_guest(host,conn,|g|g.system_message(Text::literal(format!("Encoding: {message}"))));
                }
                self.lan_load_encoding_state(host,conn);
            }
            self.lan_send_ae2_terminal(host,conn);return true;
        }
        if encoding && (0..=53).contains(&p.slot) {
            if (30..=40).contains(&p.slot) && super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    if t.processing {
                        if p.slot == 40 {
                            t.fluid_amount = 0;
                        } else {
                            let digit = (p.slot - 30) as u64;
                            if let Some(n) = t
                                .fluid_amount
                                .checked_mul(10)
                                .and_then(|n| n.checked_add(digit))
                                .filter(|n| *n <= i64::MAX as u64)
                            {
                                t.fluid_amount = n;
                            }
                        }
                    }
                }
            } else if (42..=44).contains(&p.slot)
                && super::lan_crafting::input(p.input) == Input::Pickup
            {
                let special=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).filter(|t|t.special_mode==2 && (p.slot==42 || p.slot==43)).and_then(|t|t.pattern_grid.first().and_then(Option::as_ref).map(|item|(item.clone(),t.stone_recipe.clone())));
                if let Some((item,current))=special {
                    let recipes=self.res.stations().recipes.stonecutter(&item);if !recipes.is_empty(){
                        let index=current.as_ref().and_then(|id|recipes.iter().position(|r|&r.id==id)).unwrap_or(0);
                        let next=if p.slot==42 {(index+recipes.len()-1)%recipes.len()}else{(index+1)%recipes.len()};
                        let id=recipes[next].id.clone();if let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()){t.stone_recipe=Some(id);}
                    }
                }
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    if !t.processing && p.slot == 44 {
                        t.fluid_substitutes = !t.fluid_substitutes;
                    }
                    if t.processing {
                        match p.slot {
                            42 => t.processing_page = (t.processing_page + 8) % 9,
                            43 => t.processing_page = (t.processing_page + 1) % 9,
                            44 => t.selected_fluid = t.selected_fluid.wrapping_add(1),
                            _ => {}
                        }
                    }
                }
            } else if p.slot == 45 && super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    if t.processing {
                        t.fluid_mode = !t.fluid_mode;
                    }
                }
            } else if p.slot == 46 && super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    match (t.processing,t.special_mode) {(false,0)=>t.processing=true,(true,_)=>{t.processing=false;t.special_mode=1;},(false,1)=>t.special_mode=2,_=>t.special_mode=0}
                }
            } else if p.slot == 47 && super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    t.substitutes = !t.substitutes;
                }
            } else if (0..=8).contains(&p.slot)
                && super::lan_crafting::input(p.input) == Input::Pickup
            {
                let held = host
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .and_then(|g| g.carried.clone());
                let available_fluids = self.ae2_grid_fluids(pos).unwrap_or_default();
                let allowed=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).is_some_and(|t|t.special_mode==0 || (t.special_mode==1 && p.slot<3) || (t.special_mode==2 && p.slot==0));
                if allowed {if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    let index = p.slot as usize
                        + if t.processing {
                            t.processing_page.min(8) * 9
                        } else {
                            0
                        };
                    if p.button == 1 {
                        t.pattern_grid[index] = None;
                        t.fluid_inputs[index] = None;
                    } else if t.processing && t.fluid_mode {
                        if let Some(key) = (t.fluid_amount > 0).then_some(()).and_then(|_| {
                            held.as_ref()
                                .and_then(|s| super::ae2_fluids::bucket_fluid(&s.item))
                                .map(super::ae2_fluids::fluid_key)
                                .or_else(|| {
                                    available_fluids
                                        .get(t.selected_fluid % available_fluids.len().max(1))
                                        .map(|(key, _)| key.clone())
                                })
                        }) {
                            t.fluid_inputs[index] = Some(super::ae2_processing::PatternFluid {
                                key,
                                amount: t.fluid_amount,
                            });
                            t.pattern_grid[index] = None;
                        }
                    } else {
                        t.fluid_inputs[index] = None;
                        t.pattern_grid[index] = held.map(|mut s| {
                            if !t.processing {
                                s.count = 1;
                            }
                            s
                        });
                    }
                }}
                let special_input=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).filter(|t|t.special_mode==2 && p.slot==0).and_then(|t|t.pattern_grid[0].clone());
                if let Some(item)=special_input {let selected=self.res.stations().recipes.stonecutter(&item).first().map(|r|r.id.clone());if let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()){t.stone_recipe=selected;}}
            } else if (18..=20).contains(&p.slot)
                && super::lan_crafting::input(p.input) == Input::Pickup
            {
                let held = host
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .and_then(|g| g.carried.clone());
                let available_fluids = self.ae2_grid_fluids(pos).unwrap_or_default();
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    if t.processing {
                        let index = p.slot as usize - 18 + t.processing_page.min(8) * 3;
                        if p.button == 1 {
                            t.processing_outputs[index] = None;
                            t.fluid_outputs[index] = None;
                        } else if t.fluid_mode {
                            if let Some(key) = (t.fluid_amount > 0).then_some(()).and_then(|_| {
                                held.as_ref()
                                    .and_then(|s| super::ae2_fluids::bucket_fluid(&s.item))
                                    .map(super::ae2_fluids::fluid_key)
                                    .or_else(|| {
                                        available_fluids
                                            .get(t.selected_fluid % available_fluids.len().max(1))
                                            .map(|(key, _)| key.clone())
                                    })
                            }) {
                                t.fluid_outputs[index] =
                                    Some(super::ae2_processing::PatternFluid {
                                        key,
                                        amount: t.fluid_amount,
                                    });
                                t.processing_outputs[index] = None;
                            }
                        } else {
                            t.fluid_outputs[index] = None;
                            t.processing_outputs[index] = held;
                        }
                    }
                }
            } else if p.slot == 13 && super::lan_crafting::input(p.input) == Input::Pickup {
                let (
                    grid,
                    substitutes,
                    fluid_substitutes,
                    processing,
                    special_mode,
                    stone_recipe,
                    outputs,
                    fluid_inputs,
                    fluid_outputs,
                ) = host
                    .play
                    .get(&conn)
                    .and_then(|p| p.ae2_terminal.as_ref())
                    .map(|t| {
                        (
                            t.pattern_grid.clone(),
                            t.substitutes,
                            t.fluid_substitutes,
                            t.processing,
                            t.special_mode,
                            t.stone_recipe.clone(),
                            t.processing_outputs.clone(),
                            t.fluid_inputs.clone(),
                            t.fluid_outputs.clone(),
                        )
                    })
                    .unwrap_or_default();
                let pattern = if processing {
                    if self.res.items().get("ae2:processing_pattern").is_some() {
                        // Preserve the source GenericStack ghost-slot order,
                        // including whether slot zero is an item or a fluid.
                        super::ae2_processing::ae2_encode_processing_positioned(
                            &grid,
                            &outputs,
                            &fluid_inputs,
                            &fluid_outputs,
                        )
                        .ok()
                    } else {
                        None
                    }
                } else if special_mode!=0 {
                    let count=if special_mode==1 {3}else{1};let mut special_grid=vec![None;9];special_grid[..count].clone_from_slice(&grid[..count]);
                    let recipe=if special_mode==1 {self.res.stations().recipes.smithing_recipe_result(&self.res.stations().enchantments,special_grid[0].as_ref(),special_grid[1].as_ref(),special_grid[2].as_ref()).map(|(id,_)|id.to_owned())} else {stone_recipe};
                    recipe.and_then(|id| {let kind=if special_mode==1 {"smithing"}else{"stonecutting"};let output=super::ae2_processing::special_recipe_result(self.res.stations(),kind,&id,&special_grid)?;super::ae2_processing::ae2_encode_special_pattern(kind,&id,&special_grid,&output,substitutes).ok()})
                } else {
                    self.res.recipes().find(&grid[..9], 3).and_then(|(_, out)| {
                        self.res.items().get("ae2:crafting_pattern").and_then(|_| {
                            super::ae2_processing::ae2_encode_pattern_options(
                                &grid[..9],
                                &out,
                                substitutes,
                                fluid_substitutes,
                            )
                            .ok()
                        })
                    })
                };
                if let (Some(pattern),Some(part))=(pattern,encoding_part) {
                    if let Ok(Some(mut machine))=self.ae2_part_machine(part) {
                        let eligible=machine.slots.get(1).and_then(Option::as_ref).is_some_and(|item|
                            item.item.as_ref()=="ae2:blank_pattern" || super::ae2_encoding::EncodingState::from_pattern(item).is_some());
                        let empty=machine.slots.get(1).is_some_and(Option::is_none);
                        let blank=machine.slots.first().and_then(Option::as_ref).is_some_and(|item|item.item.as_ref()=="ae2:blank_pattern" && item.count>0);
                        if eligible || (empty && blank) {
                            if empty {if let Some(item)=machine.slots[0].as_mut(){item.count-=1;if item.count==0 {machine.slots[0]=None;}}}
                            machine.slots[1]=Some(pattern);
                            if let Err(message)=self.ae2_store_part_machine(part,&machine) {self.as_guest(host,conn,|g|g.system_message(Text::literal(format!("Encoding: {message}"))));}
                        } else {self.as_guest(host,conn,|g|g.system_message(Text::literal("Insert a blank pattern into the terminal")));}
                    }
                }
            }
            if let Err(message)=self.lan_store_encoding_state(host,conn) {
                self.as_guest(host,conn,|g|g.system_message(Text::literal(format!("Encoding: {message}"))));
                self.lan_load_encoding_state(host,conn);
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if craft_amount_edit && (0..=53).contains(&p.slot) {
            if super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    match p.slot {
                        0..=9 => {
                            if let Some(n) = t
                                .craft_amount
                                .checked_mul(10)
                                .and_then(|n| n.checked_add(p.slot as u64))
                                .filter(|n| *n <= 1_000_000)
                            {
                                t.craft_amount = n;
                            }
                        }
                        10 => t.craft_amount = 0,
                        11 => {
                            if t.craft_amount > 0 {
                                t.craft_amount_edit = false;
                            }
                        }
                        _ => {}
                    }
                }
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if p.slot == 50 && super::lan_crafting::input(p.input) == Input::Pickup {
            if let Some(t) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut())
            {
                t.craft_amount_edit = true;
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if p.slot == 49 && super::lan_crafting::input(p.input) == Input::Pickup {
            let cpus = self.ae2_terminal_status_cpus(pos);
            let selected = host
                    .play
                    .get(&conn)
                    .and_then(|p| p.ae2_terminal.as_ref())
                    .map_or(0, |t| t.job_index);
                let cpu=(!cpus.is_empty()).then(||cpus[selected % cpus.len()]);
                let (part,wireless,owner)=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).map(|t|(t.craft_part,t.wireless.clone(),t.view_part.zip(t.view_identity))).unwrap_or((None,None,None));
            return self.lan_open_terminal_craft_status(host,conn,pos,cpu,part,wireless,owner);
        }
        let search_edit = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .is_some_and(|t| t.search_edit);
        if search_edit && (0..=53).contains(&p.slot) {
            if super::lan_crafting::input(p.input) == Input::Pickup {
                if let Some(t) = host
                    .play
                    .get_mut(&conn)
                    .and_then(|p| p.ae2_terminal.as_mut())
                {
                    if (0..=40).contains(&p.slot) {
                        if let Some(ch) = "abcdefghijklmnopqrstuvwxyz0123456789:_-./"
                            .chars()
                            .nth(p.slot as usize)
                        {
                            if t.search.len() < 256 {
                                t.search.push(ch);
                            }
                        }
                    } else if p.slot == 41 {
                        t.search.pop();
                    } else if p.slot == 42 {
                        t.search.clear();
                    } else if p.slot == 43 {
                        t.search_edit = false;
                    }
                    t.page = 0;
                }
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        if p.slot==52 && super::lan_crafting::input(p.input)==Input::Pickup {
            if let Some(identity)=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.chest_identity) {
                return self.lan_priority_open(host,conn,p.container,PriorityTarget::Chest{pos,identity});
            }
        }
        if matches!(p.slot, 46 | 47 | 51) && super::lan_crafting::input(p.input) == Input::Pickup {
            if let Some(identity) = host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref()).and_then(|t|t.chest_identity) {
                if matches!(p.button, 0 | 1) && self.chest_terminal_write(pos, identity, p.slot, p.button).is_none() {
                    self.lan_ae2_terminal_closed(host, conn, None);
                    host.hub.send_to(conn, &C::ContainerClose(play::ContainerClose { container: p.container }));
                    return true;
                }
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        if (p.slot == 46 || p.slot == 47 || p.slot == 50 || p.slot == 51)
            && super::lan_crafting::input(p.input) == Input::Pickup
        {
            let held = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn)
                .and_then(|g| g.carried.as_ref())
                .map(|s| s.item.to_string());
            if let Some(t) = host
                .play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut())
            {
                match p.slot {
                    46 => t.sort_mode = (t.sort_mode + 1) % 2,
                    47 => t.filter_mode = (t.filter_mode + 1) % 3,
                    51 => t.sort_descending = !t.sort_descending,
                    50 => {
                        if p.button == 1 {
                            t.search.clear();
                        } else if let Some(id) = held {
                            t.search = id;
                        } else {
                            t.search_edit = true;
                        }
                    }
                    _ => {}
                }
                t.page = 0;
            }
            self.lan_send_ae2_terminal(host, conn);
            return true;
        }
        let mut craftable = self.ae2_craftable_outputs(pos);
        craftable.extend(self.ae2_craftable_fluids(pos).iter().map(fluid_marker));
        let displayed = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_terminal.as_ref())
            .map(|t| {
                terminal_filter_entries(
                    &self.ae2_view_filtered(&t.snapshot,t.view_part),
                    &craftable,
                    t.sort_mode,
                    t.sort_descending,
                    t.filter_mode,
                    &t.search,
                    &self.pack.names,
                )
            })
            .unwrap_or_default();
        let slot = match p.slot {
            0..=53 => Some(300 + p.slot as usize),
            54..=80 => Some(9 + (p.slot as usize - 54)),
            81..=89 => Some(p.slot as usize - 81),
            _ => None,
        };
        let button = i32::from(p.button);
        let input = super::lan_crafting::input(p.input);
        // A craftable virtual entry opens confirmation without reserving inputs.
        if p.slot>=0 && (p.slot as usize)<page_size {
            let empty_cursor=host.hub.guests.iter().find(|g|g.conn==conn).is_some_and(|g|g.carried.is_none());
            if let Some((key,count))=displayed.get(page*page_size+p.slot as usize) {
                if craftable.contains(key) && (input==Input::QuickMove || (empty_cursor && *count==0)) {
                    let fluid=marker_fluid(key);
                    let is_fluid=fluid.is_some();
                    let selected=fluid.unwrap_or_else(||key.clone());
                    return self.lan_open_terminal_craft_confirm(host,conn,pos,selected,is_fluid,craft_part,wireless.clone());
                }
            }
        }
        if let Some(ctx) = &wireless {
            if slot == Some(ctx.source)
                || matches!(input, Input::Swap | Input::QuickCraft | Input::PickupAll)
            {
                self.lan_send_ae2_terminal(host, conn);
                return true;
            }
        }
        let next = self.as_guest(host, conn, |game| {
            let old_container = game.container;
            let old_page = game.ae2_terminal_page;
            game.container = Some(pos);
            game.ae2_terminal_page = page;
            let old_power = game.ae2_wireless_power;
            game.ae2_wireless_power = wireless.as_ref().map(|ctx| ctx.source);
            if let Err(message) = game.ae2_terminal_click_inner(
                slot,
                button,
                input,
                Some(&displayed),
                wireless.is_some(),
                craft_amount,
                page_size,
            ) {
                game.system_message(Text::literal(format!("ME terminal: {message}")));
            }
            let next = game.ae2_terminal_page;
            game.ae2_wireless_power = old_power;
            game.container = old_container;
            game.ae2_terminal_page = old_page;
            next
        });
        if let (Some(page), Some(t)) = (
            next,
            host.play
                .get_mut(&conn)
                .and_then(|p| p.ae2_terminal.as_mut()),
        ) {
            t.page = page;
        }
        let viewers: Vec<u32> = host
            .play
            .iter()
            .filter_map(|(&id, p)| {
                (host
                    .hub
                    .guests
                    .iter()
                    .any(|g| g.conn == id && host.hub.here(g)))
                .then(|| p.ae2_terminal.as_ref().map(|_| id))
                .flatten()
            })
            .collect();
        for viewer in viewers {
            self.lan_send_ae2_terminal(host, viewer);
        }
        true
    }
    pub(super) fn lan_ae2_terminal_closed(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let Some(_) = host.play.get_mut(&conn).and_then(|p| {
            p.ae2_terminal
                .take_if(|t| window.is_none_or(|w| t.window == w))
        }) else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
            self.guest_carried_back(host, gi);
        }
        true
    }
}

/// A physical-machine window: snapshots are for stale-click detection only.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct ProviderMenuOwner {host:u64,part:Option<[i32;4]>}
#[derive(Clone,Copy,Debug)]
enum PriorityTarget {
    Chest { pos:(i32,i32,i32), identity:u64 },
    Drive { pos:(i32,i32,i32), identity:u64 },
    Provider { pos:(i32,i32,i32), part:Option<super::ae2_parts::PartAddress>, identity:ProviderMenuOwner },
}
pub(super) struct GuestMachine {
    window: i32,
    state: i32,
    pos: (i32, i32, i32),
    part: Option<super::ae2_parts::PartAddress>,
    snapshot: Vec<Option<ItemStack>>,
    workbench_page: usize,
    owner_dim: String,
    provider_owner: Option<ProviderMenuOwner>,
    drive_owner: Option<u64>,
    priority_edit: Option<PriorityEdit>,
    link_input: Option<ItemStack>,
    link_output: Option<ItemStack>,
}
impl Game<'_> {
    /// A provider window belongs to the original physical block entity and
    /// mounted face, including when a replacement has identical slot values.
    fn ae2_provider_menu_owner(&self,pos:(i32,i32,i32),part:Option<super::ae2_parts::PartAddress>)->Result<Option<ProviderMenuOwner>,()> {
        use horizon_client::{ae2_world::MachineKind,containers::BlockEntity};
        let machine=if let Some(part)=part {self.ae2_part_machine(part).map_err(|_|())?} else {
            match self.world.block_entities.get(&pos) {Some(BlockEntity::Ae2(machine))=>Some((**machine).clone()),_=>None}
        }.ok_or(())?;
        if machine.kind!=MachineKind::PatternProvider {return Ok(None)}
        let host=self.world.block_entities.incarnation(&pos).ok_or(())?;
        let identity=if let Some(part)=part {
            let Some(BlockEntity::Ae2(machine))=self.world.block_entities.get(&pos) else{return Err(())};
            let Some(Tag::List(parts))=machine.config.get(&"parts".into()) else{return Err(())};
            let face=["east","west","up","down","south","north"].get(usize::from(part.face)).ok_or(())?;
            let mut records=parts.iter().filter(|record|record.get("face")==Some(&Tag::String((*face).into())));
            let record=records.next().ok_or(())?;if records.next().is_some(){return Err(())}
            Some(super::ae2_channels::part_identity(record).ok_or(())?)
        }else{None};
        Ok(Some(ProviderMenuOwner{host,part:identity}))
    }
    fn ae2_provider_window_valid(&self,pos:(i32,i32,i32),part:Option<super::ae2_parts::PartAddress>,original:Option<ProviderMenuOwner>)->bool {
        original.is_none() || self.ae2_provider_menu_owner(pos,part).ok().flatten()==original
    }
    fn ae2_drive_window_valid(&self,pos:(i32,i32,i32),original:Option<u64>)->bool {
        original.is_none() || (self.world.block_entities.incarnation(&pos)==original
            && matches!(self.world.block_entities.get(&pos),Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::Drive))
    }
    fn priority_target_read(&self,target:PriorityTarget)->Option<i32> {
        use horizon_client::containers::BlockEntity;
        match target {
            PriorityTarget::Chest{pos,identity} => {
                if self.chest_terminal_read(pos)?.0!=identity {return None;}
                self.ae2_storage_priority(self.ae2_local_address(pos)?).ok()
            },
            PriorityTarget::Drive{pos,identity} => {
                if !self.ae2_drive_window_valid(pos,Some(identity)){return None;}
                self.ae2_storage_priority(self.ae2_local_address(pos)?).ok()
            },
            PriorityTarget::Provider{pos,part,identity} => {
                if !self.ae2_provider_window_valid(pos,part,Some(identity)){return None;}
                let machine=if let Some(part)=part {self.ae2_part_machine(part).ok().flatten()?} else {
                    let BlockEntity::Ae2(machine)=self.world.block_entities.get(&pos)? else{return None};(**machine).clone()
                };
                match machine.config.get(&"priority".into()){None=>Some(0),Some(Tag::Int(value))=>Some(*value),_=>None}
            }
        }
    }
    fn priority_target_set(&mut self,target:PriorityTarget,old:i32,value:i32)->bool {
        if self.priority_target_read(target)!=Some(old){return false;}
        if old==value{return true;}
        use horizon_client::containers::BlockEntity;
        match target {
            PriorityTarget::Chest{pos,..}|PriorityTarget::Drive{pos,..} => self.ae2_set_storage_priority_local(pos,value).is_ok(),
            PriorityTarget::Provider{pos,part,identity:_} => {
                if let Some(part)=part {
                    let Some(mut machine)=self.ae2_part_machine(part).ok().flatten() else{return false};
                    machine.config.insert("priority".into(),Tag::Int(value));
                    if self.ae2_store_part_machine(part,&machine).is_err(){return false;}
                } else {
                    let Some(BlockEntity::Ae2(machine))=self.world.block_entities.get_mut(&pos) else{return false};
                    machine.config.insert("priority".into(),Tag::Int(value));self.world.modified.insert((pos.0>>4,pos.2>>4));
                }
                self.ae2_provider_priority_setter_local(pos,part.map(|p|p.face));true
            }
        }
    }
    fn priority_session(&self,host:&super::lan::LanHost,conn:u32)->Option<(i32,PriorityTarget,PriorityEdit)> {
        let player=host.hub.guests.iter().find(|g|g.conn==conn && host.hub.here(g) && !g.spectator())?;
        let play=host.play.get(&conn)?;
        if let Some(t)=play.ae2_terminal.as_ref() {
            let edit=t.priority_edit.clone()?;let identity=t.chest_identity?;
            if t.owner_dim!=self.dim.id() || !player.within_block_range([t.pos.0,t.pos.1,t.pos.2],4.0){return None;}
            return Some((t.window,PriorityTarget::Chest{pos:t.pos,identity},edit));
        }
        let m=play.ae2_machine.as_ref()?;let edit=m.priority_edit.clone()?;
        if m.owner_dim!=self.dim.id() || !player.within_block_range([m.pos.0,m.pos.1,m.pos.2],4.0){return None;}
        let target=if let Some(identity)=m.provider_owner {PriorityTarget::Provider{pos:m.pos,part:m.part,identity}}
            else {PriorityTarget::Drive{pos:m.pos,identity:m.drive_owner?}};
        Some((m.window,target,edit))
    }
    fn priority_payload(host:&mut super::lan::LanHost,conn:u32,window:i32,token:u64,mode:&str,value:&str) {
        if let Ok(data)=serde_json::to_vec(&serde_json::json!({"window":window,"token":token,"mode":mode,"value":value})) {
            host.hub.send_to(conn,&C::CustomPayload(horizon_protocol::packets::configuration::CustomPayload{channel:"ae2:priority_editor".into(),data}));
        }
    }
    fn lan_priority_invalid(&mut self,host:&mut super::lan::LanHost,conn:u32) {
        let window=host.play.get(&conn).and_then(|p|p.ae2_terminal.as_ref().filter(|t|t.priority_edit.is_some()).map(|t|t.window)
            .or_else(||p.ae2_machine.as_ref().filter(|m|m.priority_edit.is_some()).map(|m|m.window)));
        if let Some(window)=window {
            if !self.lan_ae2_terminal_closed(host,conn,Some(window)){self.lan_ae2_machine_closed(host,conn,Some(window));}
            host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:window}));
        }
    }
    fn lan_priority_open(&mut self,host:&mut super::lan::LanHost,conn:u32,window:i32,target:PriorityTarget)->bool {
        let Some(value)=self.priority_target_read(target) else{
            if !self.lan_ae2_terminal_closed(host,conn,Some(window)){self.lan_ae2_machine_closed(host,conn,Some(window));}
            host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:window}));return true;
        };
        if let Some(gi)=host.hub.guests.iter().position(|g|g.conn==conn){self.guest_carried_back(host,gi);}
        static NEXT_PRIORITY_EDITOR:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let token=NEXT_PRIORITY_EDITOR.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let edit=PriorityEdit{value,text:value.to_string(),token};
        if let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()).filter(|t|t.window==window){t.priority_edit=Some(edit.clone());}
        else if let Some(m)=host.play.get_mut(&conn).and_then(|p|p.ae2_machine.as_mut()).filter(|m|m.window==window){m.priority_edit=Some(edit.clone());}
        else{return true}
        Self::priority_payload(host,conn,window,edit.token,"open",&edit.text);self.lan_send_priority(host,conn);true
    }
    fn lan_send_priority(&mut self,host:&mut super::lan::LanHost,conn:u32) {
        let Some((window,target,edit))=self.priority_session(host,conn) else{self.lan_priority_invalid(host,conn);return};
        if self.priority_target_read(target).is_none(){self.lan_priority_invalid(host,conn);return;}
        let is_machine=host.play.get(&conn).and_then(|p|p.ae2_machine.as_ref()).is_some_and(|m|m.window==window);
        let Some(g)=host.hub.guests.iter().find(|g|g.conn==conn) else{return};
        let mut items=vec![None;if is_machine {27}else{54}];
        for (i,n) in [1,10,100,1000].into_iter().enumerate(){items[i]=Some(navigation(&format!("Priority +{n}; shift uses 1/16/32/64")));items[i+4]=Some(navigation(&format!("Priority -{n}; shift uses 1/16/32/64")));}
        items[8]=Some(navigation("Back to owner"));items[9]=Some(navigation(&format!("Priority: {}",edit.text)));
        items.extend((9..36).chain(0..9).map(|i|g.inventory[i].clone()));let carried=g.carried.clone();
        let snapshot=if is_machine {match target {PriorityTarget::Drive{pos,..}=>self.ae2_owned_machine_slots(pos,None),PriorityTarget::Provider{pos,part,..}=>self.ae2_owned_machine_slots(pos,part),_=>None}}else{None};
        let chest_settings=match target {PriorityTarget::Chest{pos,identity}=>self.chest_terminal_read(pos).filter(|(current,_)|*current==identity).map(|(_,settings)|settings),_=>None};
        let state=if is_machine {
            let Some(m)=host.play.get_mut(&conn).and_then(|p|p.ae2_machine.as_mut()) else{return};
            if let Some(snapshot)=snapshot {m.snapshot=snapshot;}m.state=(m.state+1)&0x7fff;m.state
        } else {
            let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()) else{return};
            if let Some(settings)=chest_settings {if (t.sort_mode,t.filter_mode,t.sort_descending)!=(settings.sort,settings.view,settings.descending){t.page=0;}t.sort_mode=settings.sort;t.filter_mode=settings.view;t.sort_descending=settings.descending;}
            t.state=(t.state+1)&0x7fff;t.state
        };
        let items=items.iter().map(|item|host.hub.stack(item)).collect();let carried=host.hub.stack(&carried);
        host.hub.send_to(conn,&C::ContainerSetContent(play::ContainerSetContent{container:window,state,items,carried}));
    }
    fn priority_update(&mut self,host:&mut super::lan::LanHost,conn:u32,text:&str,echo:bool)->bool {
        let active=host.play.get(&conn).is_some_and(|p|p.ae2_terminal.as_ref().is_some_and(|t|t.priority_edit.is_some()) || p.ae2_machine.as_ref().is_some_and(|m|m.priority_edit.is_some()));
        if !active{return false;}
        let Some((window,target,mut edit))=self.priority_session(host,conn) else{self.lan_priority_invalid(host,conn);return true};
        let Some(actual)=self.priority_target_read(target) else{self.lan_priority_invalid(host,conn);return true};
        if actual!=edit.value {
            edit.value=actual;edit.text=actual.to_string();Self::priority_payload(host,conn,window,edit.token,"set",&edit.text);
        } else {
            edit.text=text.chars().take(16).collect();
            if let Some(value)=super::ae2_number_entry::number_value(&edit.text,1,i64::from(i32::MIN),i64::MAX,'.').and_then(|n|i32::try_from(n).ok()) {
                if value!=edit.value {
                    if !self.priority_target_set(target,edit.value,value){self.lan_priority_invalid(host,conn);return true;}
                    edit.value=value;
                }
            }
            if echo {Self::priority_payload(host,conn,window,edit.token,"set",&edit.text);}
        }
        if let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()).filter(|t|t.window==window){t.priority_edit=Some(edit);}
        else if let Some(m)=host.play.get_mut(&conn).and_then(|p|p.ae2_machine.as_mut()).filter(|m|m.window==window){m.priority_edit=Some(edit);}
        self.lan_send_priority(host,conn);true
    }
    pub(super) fn lan_ae2_priority_text(&mut self,host:&mut super::lan::LanHost,conn:u32,value:&str)->bool {
        if !value.starts_with(PRIORITY_WIRE_PREFIX) {
            return host.play.get(&conn).is_some_and(|p|p.ae2_terminal.as_ref().is_some_and(|t|t.priority_edit.is_some()) || p.ae2_machine.as_ref().is_some_and(|m|m.priority_edit.is_some()));
        }
        let Some((_,_,edit))=self.priority_session(host,conn) else{self.lan_priority_invalid(host,conn);return true};
        if let Some(text)=priority_transport_for(value,edit.token) {self.priority_update(host,conn,text,false);}true
    }
    fn lan_ae2_priority_click(&mut self,host:&mut super::lan::LanHost,conn:u32,p:&play::ContainerClick)->bool {
        let Some((window,_,edit))=self.priority_session(host,conn) else{self.lan_priority_invalid(host,conn);return true};
        let input=super::lan_crafting::input(p.input);
        if p.slot==8 && input==Input::Pickup {
            if let Some(t)=host.play.get_mut(&conn).and_then(|p|p.ae2_terminal.as_mut()).filter(|t|t.window==window){t.priority_edit=None;Self::priority_payload(host,conn,window,edit.token,"close","");self.lan_send_ae2_terminal(host,conn);}
            else if let Some(m)=host.play.get_mut(&conn).and_then(|p|p.ae2_machine.as_mut()).filter(|m|m.window==window){m.priority_edit=None;Self::priority_payload(host,conn,window,edit.token,"close","");self.lan_send_ae2_machine(host,conn);}
        } else if (0..8).contains(&p.slot) && matches!(input,Input::Pickup|Input::QuickMove) {
            if let Some(next)=priority_step(&edit.text,p.slot,input==Input::QuickMove){self.priority_update(host,conn,&next,true);}
        } else {self.lan_send_priority(host,conn);}
        true
    }
    pub(super) fn ae2_has_machine_menu(&self, pos: (i32, i32, i32)) -> bool {
        use horizon_client::{ae2_world::MachineKind as K, containers::BlockEntity};
        matches!(self.world.block_entities.get(&pos),Some(BlockEntity::Ae2(m)) if matches!(m.kind,K::Drive|K::VibrationChamber|K::Inscriber|K::Interface|K::PatternProvider|K::MolecularAssembler|K::CellWorkbench|K::QuantumLink|K::SpatialIoPort|K::IoPort|K::Condenser|K::WirelessAccessPoint))
    }
    fn ae2_machine_slots(&self, pos: (i32, i32, i32)) -> Option<Vec<Option<ItemStack>>> {
        use horizon_client::containers::BlockEntity;
        match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Ae2(m)) if self.ae2_has_machine_menu(pos) => {
                let mut slots = m.slots.clone();
                slots.extend(m.upgrades.clone());
                Some(slots)
            }
            _ => None,
        }
    }
    fn ae2_owned_machine_slots(
        &self,
        pos: (i32, i32, i32),
        part: Option<super::ae2_parts::PartAddress>,
    ) -> Option<Vec<Option<ItemStack>>> {
        if part.is_none() {if let Some(slots)=self.ae2_condenser_menu(pos) {return Some(slots);}}
        let machine = if let Some(part) = part {
            self.ae2_part_machine(part).ok().flatten()?
        } else {
            match self.world.block_entities.get(&pos) {
                Some(horizon_client::containers::BlockEntity::Ae2(m))
                    if self.ae2_has_machine_menu(pos) =>
                {
                    (**m).clone()
                }
                _ => return None,
            }
        };
        let mut slots = machine.slots.clone();
        if machine.kind == horizon_client::ae2_world::MachineKind::Interface {
            for (i, entry) in super::ae2_interface::interface_fluid_stock(&machine)
                .ok()?
                .into_iter()
                .enumerate()
            {
                if let Some((key, n)) = entry {
                    if slots[i].is_some() {
                        return None;
                    }
                    let id =
                        super::ae2_fluids::fluid_bucket(&key.item_id).unwrap_or("minecraft:bucket");
                    let mut icon = ItemStack::new(id, 1);
                    icon.data_mut().custom_name = Some(format!("{}: {} mB", key.item_id, n));
                    slots[i] = Some(icon);
                }
            }
        }
        slots.extend(machine.upgrades);
        Some(slots)
    }
    pub(super) fn lan_open_ae2_part_machine(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
        part: super::ae2_parts::PartAddress,
    ) -> bool {
        let Ok(Some(machine)) = self.ae2_part_machine(part) else {
            return false;
        };
        if !matches!(
            machine.kind,
            horizon_client::ae2_world::MachineKind::Interface
                | horizon_client::ae2_world::MachineKind::PatternProvider
        ) {
            return false;
        }
        let Ok(provider_owner)=self.ae2_provider_menu_owner(pos,Some(part)) else{return false};
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
        let title = if machine.kind == horizon_client::ae2_world::MachineKind::PatternProvider {
            "ME Pattern Provider"
        } else {
            "ME Interface"
        };
        let Some(window) = host.hub.open_window(
            conn,
            "minecraft:generic_9x3",
            horizon_protocol::text::literal(title),
        ) else {
            return true;
        };
        host.play.entry(conn).or_default().ae2_machine = Some(GuestMachine {
            window,
            state: 0,
            pos,
            part: Some(part),
            snapshot: vec![],
            workbench_page: 0,
            owner_dim: self.dim.id().to_owned(),
            provider_owner,
            drive_owner: None,
            priority_edit: None,
            link_input: None,
            link_output: None,
        });
        self.lan_send_ae2_machine(host, conn);
        true
    }
    fn ae2_machine_may_place(&self, pos: (i32, i32, i32), slot: usize, item: &ItemStack) -> bool {
        use horizon_client::{ae2_world::MachineKind as K, containers::BlockEntity};
        let Some(BlockEntity::Ae2(m)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        if slot >= m.slots.len() {
            return super::ae2_machine_ui::machine_upgrade_allowed(m, slot - m.slots.len(), item);
        }
        match m.kind {
            // AE2 cells are unstackable; splitting a multi-count cell carrying
            // content components would clone its inventory.
            K::Drive => super::ae2_storage_devices::is_valid_storage_cell(item),
            K::CellWorkbench => super::ae2_storage_devices::is_valid_workbench_cell(item),
            K::VibrationChamber => {
                self.res
                    .providers()
                    .burn_time(self.res.items(), item, false)
                    > 0
            }
            K::Inscriber => slot != 3,
            K::Charger => true,
            K::QuantumLink => {
                slot == 0 && item.count == 1 && super::ae2_quantum::frequency(item).is_some()
            }
            K::SpatialIoPort => {
                slot == 0
                    && item.count == 1
                    && matches!(
                        item.item.as_ref(),
                        "ae2:spatial_storage_cell_2"
                            | "ae2:spatial_storage_cell_16"
                            | "ae2:spatial_storage_cell_128"
                    )
            }
            K::IoPort => slot < 6 && super::ae2_storage_devices::is_valid_storage_cell(item),
            K::Condenser => {
                slot == 1
                    && matches!(
                        item.item.as_ref(),
                        "ae2:cell_component_1k"
                            | "ae2:cell_component_4k"
                            | "ae2:cell_component_16k"
                            | "ae2:cell_component_64k"
                            | "ae2:cell_component_256k"
                    )
            }
            K::WirelessAccessPoint => slot == 0 && item.item.as_ref() == "ae2:wireless_booster",
            K::PatternProvider => super::ae2_pattern_access::encoded_pattern_item(item),
            K::Interface | K::MolecularAssembler => true,
            _ => false,
        }
    }
    fn ae2_machine_limit(&self, pos: (i32, i32, i32), slot: usize, item: &ItemStack) -> u32 {
        use horizon_client::{ae2_world::MachineKind as K, containers::BlockEntity};
        match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Ae2(m)) if slot >= m.slots.len() => 1,
            Some(BlockEntity::Ae2(m))
                if matches!(
                    m.kind,
                    K::Drive
                        | K::CellWorkbench
                        | K::QuantumLink
                        | K::SpatialIoPort
                        | K::IoPort
                        | K::Condenser
                ) || matches!(m.kind, K::Inscriber) && slot < 2 =>
            {
                1
            }
            _ => self
                .res
                .items()
                .get(&item.item)
                .map_or(64, |i| i.max_stack)
                .max(1),
        }
    }
    pub(super) fn lan_open_ae2_machine(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        pos: (i32, i32, i32),
    ) -> bool {
        if !self.ae2_has_machine_menu(pos) {
            return false;
        }
        let Ok(provider_owner)=self.ae2_provider_menu_owner(pos,None) else{return false};
        let drive_owner = if matches!(self.world.block_entities.get(&pos),Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::Drive) {
            let Some(identity)=self.world.block_entities.incarnation(&pos) else{return false};Some(identity)
        } else {None};
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
        let title = match self.world.block_entities.get(&pos) {
            Some(horizon_client::containers::BlockEntity::Ae2(m)) => match m.kind {
                horizon_client::ae2_world::MachineKind::Drive => "ME Drive (cells)",
                horizon_client::ae2_world::MachineKind::Inscriber => {
                    "Inscriber: top, bottom, input, output"
                }
                horizon_client::ae2_world::MachineKind::VibrationChamber => {
                    "Vibration Chamber: fuel"
                }
                horizon_client::ae2_world::MachineKind::Charger => "Charger: input",
                _ => "AE2 Machine",
            },
            _ => "AE2 Machine",
        };
        let Some(window) = host.hub.open_window(
            conn,
            "minecraft:generic_9x3",
            horizon_protocol::text::literal(title),
        ) else {
            return true;
        };
        host.play.entry(conn).or_default().ae2_machine = Some(GuestMachine {
            window,
            state: 0,
            pos,
            part: None,
            snapshot: vec![],
            workbench_page: 0,
            owner_dim: self.dim.id().to_owned(),
            provider_owner,
            drive_owner,
            priority_edit: None,
            link_input: None,
            link_output: None,
        });
        self.lan_send_ae2_machine(host, conn);
        true
    }
    fn lan_send_ae2_machine(&mut self, host: &mut super::lan::LanHost, conn: u32) {
        if host.play.get(&conn).and_then(|p|p.ae2_machine.as_ref()).is_some_and(|m|m.priority_edit.is_some()){self.lan_send_priority(host,conn);return;}
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let Some((pos, window, part, provider_owner, drive_owner)) = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_machine.as_ref())
            .map(|m| (m.pos, m.window, m.part, m.provider_owner, m.drive_owner))
        else {
            return;
        };
        let snapshot=self.ae2_owned_machine_slots(pos,part)
            .filter(|_|self.ae2_provider_window_valid(pos,part,provider_owner) && self.ae2_drive_window_valid(pos,drive_owner));
        let Some(snapshot) = snapshot else {
            self.lan_ae2_machine_closed(host, conn, None);
            host.hub.send_to(
                conn,
                &C::ContainerClose(play::ContainerClose { container: window }),
            );
            return;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        let page = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_machine.as_ref())
            .map_or(0, |m| m.workbench_page);
        let mut items = self
            .ae2_workbench_menu_slots(pos, page)
            .unwrap_or_else(|| snapshot.clone());
        items.resize(27, None);
        if part.is_none() {if let Some(condenser)=self.ae2_condenser_menu(pos) {items=condenser;}}
        let displayed_machine = if let Some(part) = part {
            self.ae2_part_machine(part).ok().flatten()
        } else {
            match self.world.block_entities.get(&pos) {
                Some(horizon_client::containers::BlockEntity::Ae2(machine)) => {
                    Some((**machine).clone())
                }
                _ => None,
            }
        };
        if let Some(machine) = displayed_machine {
            if matches!(machine.kind,horizon_client::ae2_world::MachineKind::Drive|horizon_client::ae2_world::MachineKind::PatternProvider) {
                items[26]=Some(navigation("Priority: click to edit"));
            }
            if machine.kind == horizon_client::ae2_world::MachineKind::Inscriber {
                let value=|key:&str,default:&str|match machine.config.get(&key.into()) {Some(Tag::String(s))=>s.to_string().unwrap_or_else(|_|default.to_owned()),_=>default.to_owned()};
                items[24]=Some(navigation(&format!("Auto export: {}; click to change",value("auto_export","NO"))));
                items[25]=Some(navigation(&format!("Separate sides: {}; click to change",value("inscriber_separate_sides","NO"))));
                items[26]=Some(navigation(&format!("Input capacity: {}; click to change",value("inscriber_input_capacity","SIXTY_FOUR"))));
            }
            if machine.kind == horizon_client::ae2_world::MachineKind::Interface {
                items[18] = machine.upgrades.first().cloned().flatten();
                let priority = match machine.config.get(&"priority".into()) {
                    Some(Tag::Int(n)) => *n,
                    _ => 0,
                };
                for (index, amount) in [1, 10, 100, 1000].into_iter().enumerate() {
                    items[19 + index] = Some(navigation(&format!(
                        "Priority: {priority}; left +{amount}, right -{amount}"
                    )));
                }
                let fuzzy = match machine.config.get(&"ae2_fuzzy_mode".into()) {
                    Some(Tag::String(s)) => s.to_string().unwrap_or_else(|_| "ignore_all".into()),
                    _ => "ignore_all".into(),
                };
                items[23] = Some(navigation(&format!("Fuzzy mode: {fuzzy}; click to change")));
                if let Ok(config) = super::ae2_interface::interface_config(&machine) {
                    for (index, entry) in config.into_iter().enumerate() {
                        items[9 + index] = entry;
                    }
                }
                for (offset, values) in [
                    (9, super::ae2_interface::interface_fluid_config(&machine)),
                    (0, super::ae2_interface::interface_fluid_stock(&machine)),
                ] {
                    if let Ok(values) = values {
                        for (index, entry) in values.into_iter().enumerate() {
                            if let Some((key, n)) = entry {
                                let id = super::ae2_fluids::fluid_bucket(&key.item_id)
                                    .unwrap_or("minecraft:bucket");
                                let mut icon = ItemStack::new(id, 1);
                                icon.data_mut().custom_name =
                                    Some(format!("{}: {} mB", key.item_id, n));
                                items[offset + index] = Some(icon);
                            }
                        }
                    }
                }
            }
            if machine.kind == horizon_client::ae2_world::MachineKind::PatternProvider {
                let visible=machine.config.get(&"terminal_visible".into())!=Some(&Tag::Byte(0));
                items[20]=Some(navigation(if visible {"Pattern Access: visible"} else {"Pattern Access: hidden"}));
                let enabled = machine.config.get(&"ae2_blocking".into()) == Some(&Tag::Byte(1));
                items[18] = Some(navigation(if enabled {
                    "Blocking mode: on"
                } else {
                    "Blocking mode: off"
                }));
                let mode = match machine.config.get(&"ae2_lock_mode".into()) {
                    Some(Tag::Int(n)) => *n,
                    _ => 0,
                };
                items[19] = Some(navigation(match mode {
                    1 => "Craft lock: until redstone pulse",
                    2 => "Craft lock: while redstone high",
                    3 => "Craft lock: while redstone low",
                    4 => "Craft lock: until result",
                    _ => "Craft lock: none",
                }));
            }
        }
        if matches!(self.world.block_entities.get(&pos), Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind == horizon_client::ae2_world::MachineKind::WirelessAccessPoint)
        {
            if let Some(menu) = host.play.get(&conn).and_then(|p| p.ae2_machine.as_ref()) {
                items[1] = menu.link_input.clone();
                items[2] = menu.link_output.clone();
            }
        }
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let Some(m) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.ae2_machine.as_mut())
        else {
            return;
        };
        m.snapshot = snapshot;
        m.state = (m.state + 1) & 0x7fff;
        let state = m.state;
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container: window,
            state,
            items: items.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        host.hub.send_to(conn, &packet);
    }
    pub(super) fn lan_refresh_ae2_machines(&mut self, host: &mut super::lan::LanHost) {
        let windows: Vec<_> = host
            .play
            .iter()
            .filter_map(|(&c, p)| {
                if !host
                    .hub
                    .guests
                    .iter()
                    .any(|g| g.conn == c && host.hub.here(g))
                {
                    return None;
                }
                p.ae2_machine.as_ref().map(|m| {
                    (
                        c,
                        m.pos,
                        m.window,
                        m.snapshot.clone(),
                        m.owner_dim.clone(),
                        m.part,
                        m.provider_owner,
                        m.drive_owner,
                    )
                })
            })
            .collect();
        for (conn, pos, window, old, owner_dim, part, provider_owner, drive_owner) in windows {
            let valid = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn)
                .is_some_and(|g| {
                    !g.spectator() && g.within_block_range([pos.0, pos.1, pos.2], 4.0)
                });
            if !valid || owner_dim != self.dim.id() || !self.ae2_provider_window_valid(pos,part,provider_owner) || !self.ae2_drive_window_valid(pos,drive_owner) {
                self.lan_ae2_machine_closed(host, conn, None);
                host.hub.send_to(
                    conn,
                    &horizon_protocol::packets::play::Clientbound::ContainerClose(
                        horizon_protocol::packets::play::ContainerClose { container: window },
                    ),
                );
                continue;
            }
            if self.ae2_owned_machine_slots(pos, part).as_ref() != Some(&old) {
                self.lan_send_ae2_machine(host, conn);
            }
        }
    }
    pub(super) fn lan_ae2_machine_click(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        p: &horizon_protocol::packets::play::ContainerClick,
    ) -> bool {
        let Some((pos, state, snapshot, part)) = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_machine.as_ref())
            .filter(|m| m.window == p.container)
            .map(|m| (m.pos, m.state, m.snapshot.clone(), m.part))
        else {
            return false;
        };
        let valid = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .is_some_and(|g| !g.spectator() && g.within_block_range([pos.0, pos.1, pos.2], 4.0));
        let owner_valid = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_machine.as_ref())
            .is_some_and(|m| m.owner_dim == self.dim.id() && self.ae2_provider_window_valid(pos,part,m.provider_owner) && self.ae2_drive_window_valid(pos,m.drive_owner));
        if !valid || !owner_valid || self.ae2_owned_machine_slots(pos, part).is_none() {
            self.lan_ae2_machine_closed(host, conn, None);
            host.hub.send_to(
                conn,
                &horizon_protocol::packets::play::Clientbound::ContainerClose(
                    horizon_protocol::packets::play::ContainerClose {
                        container: p.container,
                    },
                ),
            );
            return true;
        }
        if host.play.get(&conn).and_then(|p|p.ae2_machine.as_ref()).is_some_and(|m|m.priority_edit.is_some()){return self.lan_ae2_priority_click(host,conn,p);}
        if p.slot==26 && super::lan_crafting::input(p.input)==Input::Pickup {
            let target=host.play.get(&conn).and_then(|p|p.ae2_machine.as_ref()).and_then(|m|
                m.provider_owner.map(|identity|PriorityTarget::Provider{pos,part,identity})
                    .or_else(||m.drive_owner.map(|identity|PriorityTarget::Drive{pos,identity})));
            if let Some(target)=target {return self.lan_priority_open(host,conn,p.container,target);}
        }
        let paged_ghost = part.is_none()
            && matches!(self.world.block_entities.get(&pos),Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::CellWorkbench)
            && (5..23).contains(&p.slot);
        if paged_ghost && (state != p.state || self.ae2_owned_machine_slots(pos, part).as_ref() != Some(&snapshot)) {
            self.lan_send_ae2_machine(host, conn);
            return true;
        }
        if (19..=23).contains(&p.slot)
            && super::lan_crafting::input(p.input) == Input::Pickup
            && (0..=1).contains(&p.button)
        {
            let machine = if let Some(part) = part {
                self.ae2_part_machine(part).ok().flatten()
            } else {
                match self.world.block_entities.get(&pos) {
                    Some(horizon_client::containers::BlockEntity::Ae2(machine)) => {
                        Some((**machine).clone())
                    }
                    _ => None,
                }
            };
            if let Some(mut machine) =
                machine.filter(|m| m.kind == horizon_client::ae2_world::MachineKind::Interface)
            {
                if super::ae2_interface::interface_setting_click(
                    &mut machine,
                    p.slot as usize,
                    i32::from(p.button),
                ) {
                    let saved = if let Some(part) = part {
                        self.ae2_store_part_machine(part, &machine).is_ok()
                    } else if let Some(horizon_client::containers::BlockEntity::Ae2(target)) =
                        self.world.block_entities.get_mut(&pos)
                    {
                        **target = machine;
                        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                        true
                    } else {
                        false
                    };
                    if saved {
                        let viewers: Vec<_> = host
                            .play
                            .iter()
                            .filter_map(|(&c, viewer)| {
                                (viewer.ae2_machine.as_ref().is_some_and(|m| {
                                    m.pos == pos && m.part == part && m.owner_dim == self.dim.id()
                                }) && host
                                    .hub
                                    .guests
                                    .iter()
                                    .any(|g| g.conn == c && host.hub.here(g)))
                                .then_some(c)
                            })
                            .collect();
                        for viewer in viewers {
                            self.lan_send_ae2_machine(host, viewer);
                        }
                    }
                    return true;
                }
            }
        }
        if (0..9).contains(&p.slot) && super::lan_crafting::input(p.input) == Input::Pickup {
            let machine = if let Some(part) = part {
                self.ae2_part_machine(part).ok().flatten()
            } else {
                match self.world.block_entities.get(&pos) {
                    Some(horizon_client::containers::BlockEntity::Ae2(m)) => Some((**m).clone()),
                    _ => None,
                }
            };
            if let Some(mut machine) =
                machine.filter(|m| m.kind == horizon_client::ae2_world::MachineKind::Interface)
            {
                let index = p.slot as usize;
                let is_fluid = super::ae2_interface::interface_fluid_config(&machine)
                    .is_ok_and(|v| v[index].is_some())
                    || super::ae2_interface::interface_fluid_stock(&machine)
                        .is_ok_and(|v| v[index].is_some());
                if is_fluid {
                    let cursor = host
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.conn == conn)
                        .and_then(|g| g.carried.clone());
                    if let Some(cursor) = cursor {
                        if let Ok(Some(result)) = super::ae2_interface::interface_bucket_exchange(
                            &mut machine,
                            index,
                            &cursor,
                        ) {
                            let stored = if let Some(part) = part {
                                self.ae2_store_part_machine(part, &machine).is_ok()
                            } else if let Some(horizon_client::containers::BlockEntity::Ae2(
                                target,
                            )) = self.world.block_entities.get_mut(&pos)
                            {
                                **target = machine;
                                self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                                true
                            } else {
                                false
                            };
                            if stored {
                                if let Some(guest) =
                                    host.hub.guests.iter_mut().find(|g| g.conn == conn)
                                {
                                    guest.carried = Some(result);
                                }
                            }
                        }
                    }
                    self.lan_send_ae2_machine(host, conn);
                    return true;
                }
            }
        }
        if (9..18).contains(&p.slot)
            && super::lan_crafting::input(p.input) == Input::Pickup
            && (0..=1).contains(&p.button)
        {
            let machine = if let Some(part) = part {
                self.ae2_part_machine(part).ok().flatten()
            } else {
                match self.world.block_entities.get(&pos) {
                    Some(horizon_client::containers::BlockEntity::Ae2(m)) => Some((**m).clone()),
                    _ => None,
                }
            };
            if let Some(mut machine) =
                machine.filter(|m| m.kind == horizon_client::ae2_world::MachineKind::Interface)
            {
                let cursor = host
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .and_then(|g| g.carried.as_ref());
                let fluid = if p.button == 1 {
                    cursor
                        .and_then(|s| super::ae2_fluids::bucket_fluid(&s.item))
                        .map(super::ae2_fluids::fluid_key)
                } else {
                    None
                };
                let configured = if let Some(key) = fluid.as_ref() {
                    super::ae2_interface::set_interface_fluid_config(
                        &mut machine,
                        (p.slot - 9) as usize,
                        Some(key),
                    )
                } else {
                    super::ae2_interface::set_interface_config(
                        &mut machine,
                        (p.slot - 9) as usize,
                        cursor,
                    )
                };
                if configured.is_ok() {
                    if let Some(part) = part {
                        let _ = self.ae2_store_part_machine(part, &machine);
                    } else if let Some(horizon_client::containers::BlockEntity::Ae2(target)) =
                        self.world.block_entities.get_mut(&pos)
                    {
                        **target = machine;
                        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                    }
                }
                let viewers: Vec<_> = host
                    .play
                    .iter()
                    .filter_map(|(&c, p)| {
                        (p.ae2_machine.as_ref().is_some_and(|m| {
                            m.pos == pos && m.part == part && m.owner_dim == self.dim.id()
                        }) && host
                            .hub
                            .guests
                            .iter()
                            .any(|g| g.conn == c && host.hub.here(g)))
                        .then_some(c)
                    })
                    .collect();
                for viewer in viewers {
                    self.lan_send_ae2_machine(host, viewer);
                }
                return true;
            }
        }
        if let Some(part) = part {
            if (p.slot == 18 || p.slot == 19 || p.slot == 20)
                && super::lan_crafting::input(p.input) == Input::Pickup
            {
                if let Ok(Some(mut machine)) = self.ae2_part_machine(part) {
                    if machine.kind == horizon_client::ae2_world::MachineKind::PatternProvider {
                        if p.slot == 18 {
                            let old =
                                machine.config.get(&"ae2_blocking".into()) == Some(&Tag::Byte(1));
                            machine
                                .config
                                .insert("ae2_blocking".into(), Tag::Byte(i8::from(!old)));
                        } else if p.slot==20 {
                            let visible=machine.config.get(&"terminal_visible".into())!=Some(&Tag::Byte(0));
                            machine.config.insert("terminal_visible".into(),Tag::Byte(i8::from(!visible)));
                        } else {
                            let mode = match machine.config.get(&"ae2_lock_mode".into()) {
                                Some(Tag::Int(n)) => *n,
                                _ => 0,
                            };
                            machine
                                .config
                                .insert("ae2_lock_mode".into(), Tag::Int((mode + 1).rem_euclid(5)));
                            machine.config.remove(&"ae2_unlock_event".into());
                            machine.config.remove(&"ae2_unlock_stack".into());
                        }
                        let _ = self.ae2_store_part_machine(part, &machine);
                        self.lan_send_ae2_machine(host, conn);
                        return true;
                    }
                }
            }
        }
        if (p.slot == 18 || p.slot == 19 || p.slot == 20)
            && super::lan_crafting::input(p.input) == Input::Pickup
            && (0..=1).contains(&p.button)
        {
            if let Some(horizon_client::containers::BlockEntity::Ae2(machine)) =
                self.world.block_entities.get_mut(&pos)
            {
                if machine.kind == horizon_client::ae2_world::MachineKind::PatternProvider {
                    if p.slot == 18 {
                        let old = machine.config.get(&"ae2_blocking".into()) == Some(&Tag::Byte(1));
                        machine
                            .config
                            .insert("ae2_blocking".into(), Tag::Byte(i8::from(!old)));
                    } else if p.slot==20 {
                        let visible=machine.config.get(&"terminal_visible".into())!=Some(&Tag::Byte(0));
                        machine.config.insert("terminal_visible".into(),Tag::Byte(i8::from(!visible)));
                    } else {
                        let mode = match machine.config.get(&"ae2_lock_mode".into()) {
                            Some(Tag::Int(n)) => *n,
                            _ => 0,
                        };
                        machine
                            .config
                            .insert("ae2_lock_mode".into(), Tag::Int((mode + 1).rem_euclid(5)));
                        machine.config.remove(&"ae2_unlock_event".into());
                        machine.config.remove(&"ae2_unlock_stack".into());
                    }
                    self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                    let viewers: Vec<_> = host
                        .play
                        .iter()
                        .filter_map(|(&c, p)| {
                            (p.ae2_machine
                                .as_ref()
                                .is_some_and(|m| m.pos == pos && m.owner_dim == self.dim.id())
                                && host
                                    .hub
                                    .guests
                                    .iter()
                                    .any(|g| g.conn == c && host.hub.here(g)))
                            .then_some(c)
                        })
                        .collect();
                    for viewer in viewers {
                        self.lan_send_ae2_machine(host, viewer);
                    }
                    return true;
                }
            }
        }
        let wireless_access = matches!(self.world.block_entities.get(&pos), Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind == horizon_client::ae2_world::MachineKind::WirelessAccessPoint);
        if wireless_access && (p.slot == 1 || p.slot == 2) {
            let input = super::lan_crafting::input(p.input);
            if input == Input::Pickup && (0..=1).contains(&p.button) {
                if let (Some(menu), Some(guest)) = (
                    host.play
                        .get_mut(&conn)
                        .and_then(|p| p.ae2_machine.as_mut()),
                    host.hub.guests.iter_mut().find(|g| g.conn == conn),
                ) {
                    if p.slot == 1 {
                        if guest.carried.is_none() {
                            guest.carried = menu.link_input.take();
                        } else if menu.link_input.is_none()
                            && guest.carried.as_ref().is_some_and(|s| {
                                s.count == 1
                                    && matches!(
                                        s.item.as_ref(),
                                        "ae2:wireless_terminal" | "ae2:wireless_crafting_terminal"
                                    )
                            })
                        {
                            menu.link_input = guest.carried.take();
                        }
                    } else if guest.carried.is_none() {
                        guest.carried = menu.link_output.take();
                    }
                    if menu.link_output.is_none() {
                        if let Some(mut stack) = menu.link_input.take() {
                            if super::ae2_wireless::link(&mut stack, self.dim.id(), pos).is_ok() {
                                menu.link_output = Some(stack);
                            } else {
                                menu.link_input = Some(stack);
                            }
                        }
                    }
                }
            }
            self.lan_send_ae2_machine(host, conn);
            return true;
        }
        let is_interface = if let Some(part) = part {
            self.ae2_part_machine(part)
                .ok()
                .flatten()
                .is_some_and(|m| m.kind == horizon_client::ae2_world::MachineKind::Interface)
        } else {
            matches!(self.world.block_entities.get(&pos),Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::Interface)
        };
        let slot = match p.slot {
            18 if is_interface => Some(309),
            0..=26 => Some(300 + p.slot as usize),
            27..=53 => Some(9 + (p.slot as usize - 27)),
            54..=62 => Some(p.slot as usize - 54),
            _ => None,
        };
        let input = super::lan_crafting::input(p.input);
        let button = i32::from(p.button);
        let mut workbench_page = host
            .play
            .get(&conn)
            .and_then(|p| p.ae2_machine.as_ref())
            .map_or(0, |m| m.workbench_page);
        let provider_slots = if let Some(part) = part {
            self.ae2_part_machine(part).ok().flatten()
        } else {
            match self.world.block_entities.get(&pos) {
                Some(horizon_client::containers::BlockEntity::Ae2(machine)) => Some((**machine).clone()),
                _ => None,
            }
        }.filter(|machine|machine.kind==horizon_client::ae2_world::MachineKind::PatternProvider)
            .map(|machine|machine.slots);
        let is_provider=provider_slots.is_some();
        let provider_setter=self.as_guest(host, conn, |g| {
            if is_provider && input==Input::Swap {
                return g.ae2_provider_slot_swap(pos,part,slot,button);
            }
            if let Some(part) = part {
                g.ae2_part_machine_click(part, slot, button, input);
            } else if !g.ae2_workbench_menu_click(pos, &mut workbench_page, slot, button, input) {
                g.ae2_machine_click(pos, slot, button, input);
            }
            false
        }).unwrap_or(false);
        if let Some(before)=provider_slots {
            let after=if let Some(part)=part {
                self.ae2_part_machine(part).ok().flatten()
            } else {
                match self.world.block_entities.get(&pos) {
                    Some(horizon_client::containers::BlockEntity::Ae2(machine))=>Some((**machine).clone()),
                    _=>None,
                }
            };
            if after.is_some_and(|machine|machine.kind==horizon_client::ae2_world::MachineKind::PatternProvider && (machine.slots!=before || provider_setter)) {
                if let Some(part)=part {
                    self.ae2_provider_patterns_changed(part.host,Some(part.face));
                } else {
                    self.ae2_provider_patterns_changed_local(pos,None);
                }
            }
        }
        if is_provider && input==Input::Swap {
            if let Some(gi)=host.hub.guests.iter().position(|guest|guest.conn==conn) {host.hub.send_inventory(gi);}
        }
        if let Some(m) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.ae2_machine.as_mut())
        {
            m.workbench_page = workbench_page;
        }
        let viewers: Vec<_> = host
            .play
            .iter()
            .filter_map(|(&c, p)| {
                (host
                    .hub
                    .guests
                    .iter()
                    .any(|g| g.conn == c && host.hub.here(g)))
                .then(|| p.ae2_machine.as_ref().map(|_| c))
                .flatten()
            })
            .collect();
        for c in viewers {
            self.lan_send_ae2_machine(host, c);
        }
        true
    }
    /// Source SWAP writes the physical slot even when two encoded stacks
    /// compare equal. Report that setter separately from value differences so
    /// the provider registry receives exactly one post-commit refresh.
    fn ae2_provider_slot_swap(&mut self,pos:(i32,i32,i32),part:Option<super::ae2_parts::PartAddress>,slot:Option<usize>,button:i32)->bool {
        use horizon_client::{ae2_world::MachineKind,containers::BlockEntity};
        let Some(index)=slot.and_then(|slot|slot.checked_sub(300)).filter(|index|*index<9) else{return false};
        if !(0..=8).contains(&button) && button!=40 {return false}
        let mut machine=if let Some(part)=part {self.ae2_part_machine(part).ok().flatten()} else {
            match self.world.block_entities.get(&pos) {Some(BlockEntity::Ae2(machine))=>Some((**machine).clone()),_=>None}
        };
        let Some(machine)=machine.as_mut().filter(|machine|machine.kind==MachineKind::PatternProvider && machine.slots.len()==9) else{return false};
        let source=if button==40 {self.offhand.clone()} else {self.inventory[button as usize].clone()};
        let target=machine.slots[index].clone();
        if source.is_none() && target.is_none() {return false}
        // Real encoded items stack to one. Reject malformed overstacks before
        // mutation; the general oversized-stack/drop path remains separate.
        if source.as_ref().is_some_and(|stack|stack.count!=1 || !super::ae2_pattern_access::encoded_pattern_item(stack))
            || target.as_ref().is_some_and(|stack|stack.count!=1) {return false}
        machine.slots[index]=source;
        if let Some(part)=part {
            if self.ae2_store_part_machine(part,machine).is_err() {return false}
        } else {
            let Some(BlockEntity::Ae2(current))=self.world.block_entities.get_mut(&pos) else{return false};
            if current.kind!=MachineKind::PatternProvider {return false}
            **current=machine.clone();self.world.modified.insert((pos.0>>4,pos.2>>4));
        }
        if button==40 {self.offhand=target;} else {self.inventory[button as usize]=target;}
        true
    }
    /// Per-face part inventory transaction. Stale menu snapshots are checked
    /// before this runs; the player and owned part roll back together on error.
    fn ae2_part_machine_click(
        &mut self,
        part: super::ae2_parts::PartAddress,
        slot: Option<usize>,
        button: i32,
        input: Input,
    ) {
        let Some(slot) = slot else { return };
        let Ok(Some(mut machine)) = self.ae2_part_machine(part) else {
            return;
        };
        let before_inventory = self.inventory.clone();
        let before_carried = self.carried.clone();
        let normal = machine.slots.len();
        let mut slots = machine.slots.clone();
        slots.extend(machine.upgrades.clone());
        if (300..327).contains(&slot) {
            let i = slot - 300;
            if i >= slots.len() {
                return;
            }
            if input == Input::QuickMove {
                if let Some(item) = slots[i].take() {
                    slots[i] = self.give(item);
                }
            } else if input == Input::Pickup && (0..=1).contains(&button) {
                let held = self.carried.take();
                let item = slots[i].take();
                let allows = |s: &ItemStack| {
                    if i >= normal {
                        super::ae2_machine_ui::machine_upgrade_allowed(&machine, i - normal, s)
                    } else {
                        machine.kind != horizon_client::ae2_world::MachineKind::PatternProvider
                            || super::ae2_pattern_access::encoded_pattern_item(s)
                    }
                };
                let max = |s: &ItemStack| {
                    if i >= normal {
                        1
                    } else {
                        self.res
                            .items()
                            .get(&s.item)
                            .map_or(64, |v| v.max_stack)
                            .max(1)
                    }
                };
                match (held, item, button) {
                    (None, None, _) => {}
                    (None, Some(v), 0) => self.carried = Some(v),
                    (None, Some(mut v), 1) => {
                        let n = v.count.div_ceil(2);
                        v.count -= n;
                        self.carried = Some(ItemStack {
                            count: n,
                            ..v.clone()
                        });
                        if v.count > 0 {
                            slots[i] = Some(v);
                        }
                    }
                    (Some(mut h), None, b) if allows(&h) => {
                        let n = max(&h).min(if b == 1 { 1 } else { h.count });
                        slots[i] = Some(ItemStack {
                            count: n,
                            ..h.clone()
                        });
                        h.count -= n;
                        if h.count > 0 {
                            self.carried = Some(h);
                        }
                    }
                    (Some(mut h), Some(mut v), b) if same(&h, &v) && allows(&h) => {
                        let n =
                            max(&v)
                                .saturating_sub(v.count)
                                .min(if b == 1 { 1 } else { h.count });
                        v.count += n;
                        h.count -= n;
                        slots[i] = Some(v);
                        if h.count > 0 {
                            self.carried = Some(h);
                        }
                    }
                    (Some(h), Some(v), _) if allows(&h) && h.count <= max(&h) => {
                        slots[i] = Some(h);
                        self.carried = Some(v);
                    }
                    (h, v, _) => {
                        self.carried = h;
                        slots[i] = v;
                    }
                }
            } else {
                return;
            }
        } else if slot < 36 {
            if input == Input::QuickMove {
                if let Some(mut held) = self.inventory[slot].take() {
                    for i in (normal..slots.len()).chain(0..normal) {
                        let allowed = if i >= normal {
                            super::ae2_machine_ui::machine_upgrade_allowed(
                                &machine,
                                i - normal,
                                &held,
                            )
                        } else {
                            machine.kind != horizon_client::ae2_world::MachineKind::PatternProvider
                                || super::ae2_pattern_access::encoded_pattern_item(&held)
                        };
                        if !allowed {
                            continue;
                        }
                        let max = if i >= normal {
                            1
                        } else {
                            self.res
                                .items()
                                .get(&held.item)
                                .map_or(64, |v| v.max_stack)
                                .max(1)
                        };
                        match &mut slots[i] {
                            Some(v) if same(v, &held) => {
                                let n = max.saturating_sub(v.count).min(held.count);
                                v.count += n;
                                held.count -= n
                            }
                            None => {
                                let n = max.min(held.count);
                                slots[i] = Some(ItemStack {
                                    count: n,
                                    ..held.clone()
                                });
                                held.count -= n
                            }
                            _ => {}
                        }
                        if held.count == 0 {
                            break;
                        }
                    }
                    if held.count > 0 {
                        self.inventory[slot] = Some(held)
                    }
                }
            } else if input == Input::Pickup && (0..=1).contains(&button) {
                let held = self.carried.take();
                let item = self.inventory[slot].take();
                match (held, item, button) {
                    (None, None, _) => {}
                    (None, Some(v), 0) => self.carried = Some(v),
                    (None, Some(mut v), 1) => {
                        let n = v.count.div_ceil(2);
                        v.count -= n;
                        self.carried = Some(ItemStack {
                            count: n,
                            ..v.clone()
                        });
                        if v.count > 0 {
                            self.inventory[slot] = Some(v)
                        }
                    }
                    (Some(h), None, 0) => self.inventory[slot] = Some(h),
                    (Some(mut h), None, 1) => {
                        self.inventory[slot] = Some(ItemStack {
                            count: 1,
                            ..h.clone()
                        });
                        h.count -= 1;
                        if h.count > 0 {
                            self.carried = Some(h)
                        }
                    }
                    (Some(mut h), Some(mut v), b) if same(&h, &v) => {
                        let max = self
                            .res
                            .items()
                            .get(&v.item)
                            .map_or(64, |i| i.max_stack)
                            .max(1);
                        let n = max
                            .saturating_sub(v.count)
                            .min(if b == 1 { 1 } else { h.count });
                        v.count += n;
                        h.count -= n;
                        self.inventory[slot] = Some(v);
                        if h.count > 0 {
                            self.carried = Some(h)
                        }
                    }
                    (Some(h), Some(v), _) => {
                        self.inventory[slot] = Some(h);
                        self.carried = Some(v)
                    }
                    (h, v, _) => {
                        self.carried = h;
                        self.inventory[slot] = v
                    }
                }
            } else {
                return;
            }
        } else {
            return;
        }
        let had_crafting_card = machine
            .upgrades
            .iter()
            .flatten()
            .any(|s| s.item.as_ref() == "ae2:crafting_card");
        machine.slots = slots[..normal].to_vec();
        machine.upgrades = slots[normal..].to_vec();
        if had_crafting_card
            && !machine
                .upgrades
                .iter()
                .flatten()
                .any(|s| s.item.as_ref() == "ae2:crafting_card")
        {
            super::ae2_interface::mark_card_removed(&mut machine);
        }
        super::ae2_interface::wake_interface(&mut machine);
        if self.ae2_store_part_machine(part, &machine).is_err() {
            self.inventory = before_inventory;
            self.carried = before_carried;
        }
    }
    fn ae2_machine_click(&mut self,pos:(i32,i32,i32),slot:Option<usize>,button:i32,input:Input) {
        if matches!(slot,Some(324|325|326)) && input==Input::Pickup && (0..=1).contains(&button)
            && self.ae2_inscriber_setting(pos,slot.unwrap(),button==1) {return;}
        let condenser=matches!(self.world.block_entities.get(&pos),Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::Condenser);
        if !condenser {
            let before=match self.world.block_entities.get(&pos) {Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::QuantumLink=>Some(m.slots.first().cloned()),_=>None};
            let workbench=match self.world.block_entities.get(&pos) {
                Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::CellWorkbench=>Some((**m).clone()),
                _=>None,
            };
            let player=workbench.as_ref().map(|_|(self.inventory.clone(),self.carried.clone()));
            self.ae2_machine_click_physical(pos,slot,button,input);
            if let Some(previous)=workbench {
                let after=match self.world.block_entities.get(&pos) {
                    Some(horizon_client::containers::BlockEntity::Ae2(m)) if m.kind==horizon_client::ae2_world::MachineKind::CellWorkbench=>Some((**m).clone()),
                    _=>None,
                };
                if let Some(mut next)=after.filter(|m|m.slots.first()!=previous.slots.first()) {
                    if super::ae2_storage_devices::workbench_cell_changed(&previous,&mut next).is_ok() {
                        if let Some(horizon_client::containers::BlockEntity::Ae2(m))=self.world.block_entities.get_mut(&pos) {
                            **m=next;
                            if let Some(cell)=m.slots.first().and_then(Option::as_ref){horizon_client::ae2_stack_identity::synchronize_tracked(cell);}
                        }
                    } else {
                        if let Some(horizon_client::containers::BlockEntity::Ae2(m))=self.world.block_entities.get_mut(&pos){**m=previous;}
                        if let Some((inventory,carried))=player {self.inventory=inventory;self.carried=carried;}
                    }
                }
            }
            if let Some(before)=before {
                let after=match self.world.block_entities.get(&pos) {Some(horizon_client::containers::BlockEntity::Ae2(m))=>m.slots.first().cloned(),_=>None};
                if before!=after {self.ae2_quantum_inventory_changed(pos);}
            }
            return;
        }
        let Some(slot)=slot else {return;};
        match slot {
            300 if input==Input::Pickup && (0..=1).contains(&button)=>self.ae2_condenser_cursor_input(pos,button==1),
            301=>{
                let before=match self.world.block_entities.get(&pos) {Some(horizon_client::containers::BlockEntity::Ae2(m))=>m.slots[0].clone(),_=>None};
                self.ae2_machine_click_physical(pos,Some(300),button,input);
                let after=match self.world.block_entities.get(&pos) {Some(horizon_client::containers::BlockEntity::Ae2(m))=>m.slots[0].clone(),_=>None};
                if before!=after {self.ae2_condenser_output_changed(pos);}
            }
            302=>self.ae2_machine_click_physical(pos,Some(301),button,input),
            326 if input==Input::Pickup && (0..=1).contains(&button)=>self.ae2_condenser_cycle(pos,button==1),
            0..36=>{
                if input!=Input::QuickMove || !self.ae2_condenser_shift_input(pos,slot) {self.ae2_machine_click_physical(pos,Some(slot),button,input);}
            }
            _=>{}
        }
    }
    fn ae2_machine_click_physical(
        &mut self,
        pos: (i32, i32, i32),
        slot: Option<usize>,
        button: i32,
        input: Input,
    ) {
        use horizon_client::containers::BlockEntity;
        let Some(slot) = slot else { return };
        let Some(mut slots) = self.ae2_machine_slots(pos) else {
            return;
        };
        if (300..327).contains(&slot) {
            let i = slot - 300;
            if i >= slots.len() {
                return;
            }
            if input == Input::QuickMove {
                if let Some(item) = slots[i].take() {
                    let rest = self.give(item);
                    slots[i] = rest;
                }
            } else if input == Input::Pickup && (0..=1).contains(&button) {
                let held = self.carried.take();
                let item = slots[i].take();
                match (held, item, button) {
                    (None, None, _) => {}
                    (None, Some(item), 0) => self.carried = Some(item),
                    (None, Some(mut item), 1) => {
                        let n = item.count.div_ceil(2);
                        item.count -= n;
                        self.carried = Some(ItemStack {
                            count: n,
                            ..item.clone()
                        });
                        if item.count > 0 {
                            slots[i] = Some(item)
                        }
                    }
                    (Some(mut held), None, b) if self.ae2_machine_may_place(pos, i, &held) => {
                        let n = self.ae2_machine_limit(pos, i, &held).min(if b == 1 {
                            1
                        } else {
                            held.count
                        });
                        slots[i] = Some(ItemStack {
                            count: n,
                            ..held.clone()
                        });
                        held.count -= n;
                        if held.count > 0 {
                            self.carried = Some(held)
                        }
                    }
                    (Some(mut held), Some(mut item), b)
                        if same(&held, &item) && self.ae2_machine_may_place(pos, i, &held) =>
                    {
                        let n = self
                            .ae2_machine_limit(pos, i, &held)
                            .saturating_sub(item.count)
                            .min(if b == 1 { 1 } else { held.count });
                        item.count += n;
                        held.count -= n;
                        slots[i] = Some(item);
                        if held.count > 0 {
                            self.carried = Some(held)
                        }
                    }
                    (Some(held), Some(item), _)
                        if self.ae2_machine_may_place(pos, i, &held)
                            && held.count <= self.ae2_machine_limit(pos, i, &held) =>
                    {
                        slots[i] = Some(held);
                        self.carried = Some(item)
                    }
                    (held, item, _) => {
                        self.carried = held;
                        slots[i] = item;
                    }
                }
            } else {
                return;
            }
            if let Some(BlockEntity::Ae2(m)) = self.world.block_entities.get_mut(&pos) {
                let n = m.slots.len();
                if slots.len() != n + m.upgrades.len() {
                    return;
                }
                let had_crafting_card = m
                    .upgrades
                    .iter()
                    .flatten()
                    .any(|s| s.item.as_ref() == "ae2:crafting_card");
                m.slots = slots[..n].to_vec();
                m.upgrades = slots[n..].to_vec();
                if had_crafting_card
                    && !m
                        .upgrades
                        .iter()
                        .flatten()
                        .any(|s| s.item.as_ref() == "ae2:crafting_card")
                {
                    super::ae2_interface::mark_card_removed(m);
                }
                super::ae2_interface::wake_interface(m);
                self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            }
            return;
        }
        if slot >= 36 {
            return;
        }
        if input == Input::QuickMove {
            if let Some(mut held) = self.inventory[slot].take() {
                if let Some(BlockEntity::Ae2(machine)) = self.world.block_entities.get_mut(&pos) {
                    if super::ae2_machine_ui::machine_upgrade_quick_insert(machine, &mut held) > 0 {
                        self.inventory[slot] = (held.count > 0).then_some(held);
                        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                        return;
                    }
                }
                for i in 0..slots.len() {
                    if !self.ae2_machine_may_place(pos, i, &held) {
                        continue;
                    }
                    let max = self.ae2_machine_limit(pos, i, &held);
                    match &mut slots[i] {
                        Some(item) if same(item, &held) => {
                            let n = max.saturating_sub(item.count).min(held.count);
                            item.count += n;
                            held.count -= n
                        }
                        None => {
                            let n = max.min(held.count);
                            slots[i] = Some(ItemStack {
                                count: n,
                                ..held.clone()
                            });
                            held.count -= n
                        }
                        _ => {}
                    }
                    if held.count == 0 {
                        break;
                    }
                }
                if held.count > 0 {
                    self.inventory[slot] = Some(held);
                }
                if let Some(BlockEntity::Ae2(m)) = self.world.block_entities.get_mut(&pos) {
                    let n = m.slots.len();
                    if slots.len() != n + m.upgrades.len() {
                        return;
                    }
                    let had_crafting_card = m
                        .upgrades
                        .iter()
                        .flatten()
                        .any(|s| s.item.as_ref() == "ae2:crafting_card");
                    m.slots = slots[..n].to_vec();
                    m.upgrades = slots[n..].to_vec();
                    if had_crafting_card
                        && !m
                            .upgrades
                            .iter()
                            .flatten()
                            .any(|s| s.item.as_ref() == "ae2:crafting_card")
                    {
                        super::ae2_interface::mark_card_removed(m);
                    }
                    super::ae2_interface::wake_interface(m);
                    self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
                }
            }
            return;
        }
        if input != Input::Pickup || !(0..=1).contains(&button) {
            return;
        }
        let held = self.carried.take();
        let item = self.inventory[slot].take();
        match (held, item, button) {
            (None, None, _) => {}
            (None, Some(item), 0) => self.carried = Some(item),
            (None, Some(mut item), 1) => {
                let n = item.count.div_ceil(2);
                item.count -= n;
                self.carried = Some(ItemStack {
                    count: n,
                    ..item.clone()
                });
                if item.count > 0 {
                    self.inventory[slot] = Some(item)
                }
            }
            (Some(held), None, 0) => self.inventory[slot] = Some(held),
            (Some(mut held), None, 1) => {
                self.inventory[slot] = Some(ItemStack {
                    count: 1,
                    ..held.clone()
                });
                held.count -= 1;
                if held.count > 0 {
                    self.carried = Some(held)
                }
            }
            (Some(mut held), Some(mut item), b) if same(&held, &item) => {
                let max = self
                    .res
                    .items()
                    .get(&item.item)
                    .map_or(64, |i| i.max_stack)
                    .max(1);
                let n = max
                    .saturating_sub(item.count)
                    .min(if b == 1 { 1 } else { held.count });
                item.count += n;
                held.count -= n;
                self.inventory[slot] = Some(item);
                if held.count > 0 {
                    self.carried = Some(held)
                }
            }
            (Some(held), Some(item), _) => {
                self.inventory[slot] = Some(held);
                self.carried = Some(item)
            }
            (held, item, _) => {
                self.carried = held;
                self.inventory[slot] = item
            }
        }
    }
    pub(super) fn lan_ae2_machine_closed(
        &mut self,
        host: &mut super::lan::LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let Some(menu) = host.play.get_mut(&conn).and_then(|p| {
            p.ae2_machine
                .take_if(|m| window.is_none_or(|w| m.window == w))
        }) else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
            self.guest_carried_back(host, gi);
            let items: Vec<_> = [menu.link_input, menu.link_output]
                .into_iter()
                .flatten()
                .collect();
            self.drop_guest_items(host, gi, items);
            // A rejected stale SWAP may have predicted an inventory edit even
            // with an empty cursor. Restore the actual player slots on close.
            host.hub.send_inventory(gi);
        }
        true
    }
}

#[cfg(test)]
mod terminal_filter_tests {
    use super::*;
    #[test]
    fn wireless_gesture_snapshot_allows_battery_ticks_but_rejects_other_changes() {
        let mut inventory = vec![None; 36];
        inventory[0] = Some(ItemStack::new("ae2:wireless_crafting_terminal", 1));
        inventory[1] = Some(ItemStack::new("minecraft:stone", 3));
        let mut after = inventory.clone();
        after[0].as_mut().unwrap().data_mut().set_other("minecraft:custom_data", Some(Tag::compound([
            ("ae2_wireless_energy", Tag::Double(42.0f64.to_bits()))
        ])));
        assert!(wireless_unchanged_inventory(0, &inventory, &after, &None, &None));
        after[1].as_mut().unwrap().count = 2;
        assert!(!wireless_unchanged_inventory(0, &inventory, &after, &None, &None));
        let offhand = Some(ItemStack::new("ae2:wireless_terminal", 1));
        let mut changed = offhand.clone(); changed.as_mut().unwrap().damage = 1;
        assert!(wireless_unchanged_inventory(40, &inventory, &inventory, &offhand, &changed));
        assert!(!wireless_unchanged_inventory(0, &inventory, &inventory, &offhand, &changed));
    }
    #[test]
    fn number_entry_priority_expression_and_steps() {
        assert!(priority_confirmable("=2^3^4"));assert!(!priority_confirmable("1/3"));
        assert_eq!(priority_step("1+2",1,false).as_deref(),Some("13"));
        assert_eq!(super::super::ae2_number_entry::number_value("=3*8",1,i64::from(i32::MIN),i64::MAX,'.').and_then(|v|i32::try_from(v).ok()),Some(24));
        assert_eq!(super::super::ae2_number_entry::number_value("2147483648",1,i64::from(i32::MIN),i64::MAX,'.').and_then(|v|i32::try_from(v).ok()),None);
    }
    #[test]
    fn priority_editor_steps_follow_source_boundaries_and_special_one() {
        assert_eq!(priority_step("1",1,false).as_deref(),Some("10"));
        assert_eq!(priority_step("1",1,true).as_deref(),Some("16"));
        assert_eq!(priority_step("-2147483648",4,false).as_deref(),Some("-2147483648"));
        assert_eq!(priority_step("2147483647",0,false).as_deref(),Some("2147483648"));
        assert_eq!(priority_step("9999999999999999",3,false).unwrap().len(),16);
        assert!("2147483648".parse::<i32>().is_err());assert_eq!("-2147483648".parse::<i32>(),Ok(i32::MIN));
        assert!(priority_confirmable("2147483648"));assert!(priority_confirmable("-2147483648"));
        assert!(!priority_confirmable("-2147483649"));assert!(!priority_confirmable("-"));
        let old=priority_transport_value(7,"-37");assert_eq!(priority_transport_for(&old,7),Some("-37"));
        assert_eq!(priority_transport_for(&old,8),None,"a prior editor cannot update its replacement");
        assert!(priority_transport_parse(&horizon_client::clipboard::filter_text(&old,false)).is_none(),"ordinary editable text cannot use the internal envelope");
        assert!(priority_transport_parse("\u{1f}ae2p:not-a-token:4").is_none());assert!(priority_transport_value(u64::MAX,"-2147483648").len()<=50);
        assert_eq!(priority_step("-",0,false).as_deref(),Some("1"));assert_eq!(priority_step("0",8,false),None);
        assert!(priority_confirmable("=2^3^4"));assert!(!priority_confirmable("1/3"));
        assert_eq!(priority_step("1+2",1,false).as_deref(),Some("13"));
    }
    #[test]
    fn chest_terminal_settings_preserve_other_state_and_cycle_both_directions() {
        use horizon_client::ae2_world::{Machine, MachineKind};
        let mut chest = Machine::new(MachineKind::Chest);
        chest.config.insert("priority".into(), Tag::Int(17));
        chest.config.insert("sort_by".into(), Tag::String("invalid".into()));
        assert_eq!(chest_terminal_settings(&chest), ChestTerminalSettings { sort: 0, view: 0, descending: false });
        assert!(chest_terminal_setting_click(&mut chest, 46, 1));
        assert_eq!(chest.config.get(&"sort_by".into()), Some(&Tag::String("mod".into())));
        assert!(chest_terminal_setting_click(&mut chest, 47, 1));
        assert_eq!(chest.config.get(&"view_mode".into()), Some(&Tag::String("craftable".into())));
        assert!(chest_terminal_setting_click(&mut chest, 51, 0));
        assert_eq!(chest_terminal_settings(&chest), ChestTerminalSettings { sort: 2, view: 2, descending: true });
        assert_eq!(chest.config.get(&"priority".into()), Some(&Tag::Int(17)));
        let unchanged=chest.clone();assert!(!chest_terminal_setting_click(&mut chest, 46, 2));assert_eq!(chest, unchanged);
        let mut other=Machine::new(MachineKind::Drive);assert!(!chest_terminal_setting_click(&mut other, 46, 0));
        let original=ChestTerminalSettings{sort:0,view:0,descending:false};let updated=chest_terminal_settings(&chest);
        let revisions=std::collections::VecDeque::from([(4,original),(5,original),(6,updated)]);
        assert!(chest_revision_matches(&revisions,4,original),"unchanged older views stay usable");
        assert!(!chest_revision_matches(&revisions,4,updated),"queued settings click cannot overwrite a newer Card import");
        assert!(chest_revision_matches(&revisions,6,updated));assert!(!chest_revision_matches(&revisions,3,updated));
    }
    #[test]
    fn terminal_toolbox_rejects_preexisting_overstack_before_staging() {
        use horizon_client::ae2_network_tool::Inventory;
        let mut pocket = Inventory { slots: std::array::from_fn(|_| None) };
        pocket.slots[0] = Some(ItemStack::new("ae2:speed_card",64));
        // Existing non-card contents remain readable and extractable.
        pocket.slots[1] = Some(ItemStack::new("minecraft:diamond",7));
        assert!(toolbox_staging_safe(&pocket, |_| 64));
        pocket.slots[0].as_mut().unwrap().count = 65;
        assert!(!toolbox_staging_safe(&pocket, |_| 64));
        pocket.slots[0].as_mut().unwrap().count = 64;
        pocket.slots[1].as_mut().unwrap().count = 100;
        assert!(!toolbox_staging_safe(&pocket, |_| 200));
    }
    #[test]
    fn toolbox_wireless_slots_do_not_require_a_view_cell_panel() {
        let wireless=terminal_visible_slots(false,true);
        assert!((365..374).all(|slot|wireless.contains(&slot)));
        assert!((360..365).all(|slot|!wireless.contains(&slot)));
        let physical=terminal_visible_slots(true,true);
        assert!((360..374).all(|slot|physical.contains(&slot)));
        let no_toolbox=terminal_visible_slots(true,false);
        assert!((360..365).all(|slot|no_toolbox.contains(&slot)));
        assert!((365..374).all(|slot|!no_toolbox.contains(&slot)));
    }
    #[test]
    fn toolbox_wireless_owners_and_mixed_drag_are_protected() {
        use horizon_client::{banner::Banners, brewing::Brewing, crafting::Recipes,
            items::Items, menu::{Env, Kind, Menu}};
        assert!(wireless_toolbox_swap_refused(40,None,1));
        assert!(wireless_toolbox_swap_refused(0,Some(40),1));
        assert!(wireless_toolbox_swap_refused(0,Some(8),8));
        assert!(wireless_toolbox_swap_refused(0,Some(8),0));
        assert!(!wireless_toolbox_swap_refused(0,Some(8),3));
        assert_eq!(toolbox_drag_slot(95,true,8,Some(0)),Some(0));
        assert_eq!(toolbox_drag_slot(27,true,8,Some(0)),Some(9));
        assert_eq!(toolbox_drag_slot(81,true,8,Some(0)),None);
        assert_eq!(toolbox_drag_slot(89,true,8,Some(0)),None);
        assert_eq!(toolbox_drag_slot(0,true,8,Some(0)),None);
        assert!(toolbox_pickup_locked(9,8,Some(0)));
        assert!(toolbox_pickup_locked(17,8,Some(0)));
        assert!(!toolbox_pickup_locked(0,8,Some(0)));
        assert!(!toolbox_pickup_locked(45,8,Some(0)));
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.local/resources/26.3");
        assert!(root.join("items.json").exists(), "native test resources are required");
        let items=Items::catalog(&root).unwrap(); let recipes=Recipes::load(&root,&items).unwrap();
        let brewing=Brewing::load(&root,&items); let banners=Banners::load(&root);
        let stations=horizon_client::stations::StationRecipes::load(&root,&items);
        let enchantments=horizon_client::enchant::Enchantments::default();
        let names=horizon_client::creative::Names::default();
        let env=Env{items:&items,recipes:&recipes,brewing:&brewing,banners:&banners,
            station_recipes:&stations,enchantments:&enchantments,names:&names,trade:None,maps:None};
        let mut menu=Menu::new(Kind::Chest(2));menu.network_tool_slot=Some(8);menu.network_tool_main_limit=Some(9);
        menu.set_player_item(0,Some(ItemStack::new("ae2:wireless_crafting_terminal",1)));
        menu.set_player_item(8,Some(ItemStack::new("ae2:network_tool",1)));
        menu.carried=Some(ItemStack::new("ae2:speed_card",6));
        menu.click(&env,-999,0,Input::QuickCraft).unwrap();
        for slot in [95,27,81,89,0] {if let Some(index)=toolbox_drag_slot(slot,true,8,Some(0)) {menu.click(&env,index,1,Input::QuickCraft).unwrap();}}
        menu.click(&env,-999,2,Input::QuickCraft).unwrap();
        assert_eq!(menu.slot(0).map(|s|s.count),Some(3));assert_eq!(menu.slot(9).map(|s|s.count),Some(3));
        assert!(menu.carried.is_none());assert_eq!(menu.player_item(0).map(|s|s.item.as_ref()),Some("ae2:wireless_crafting_terminal"));
        assert_eq!(menu.player_item(8).map(|s|s.item.as_ref()),Some("ae2:network_tool"));
    }
    #[test]
    fn terminal_toolbox_menu_respects_source_slot_rules() {
        use horizon_client::{banner::Banners, brewing::Brewing, crafting::Recipes,
            items::Items, menu::{Env, Kind, Menu}};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.local/resources/26.3");
        assert!(root.join("items.json").exists(), "native test resources are required");
        let items=Items::catalog(&root).unwrap(); let recipes=Recipes::load(&root,&items).unwrap();
        let brewing=Brewing::load(&root,&items); let banners=Banners::load(&root);
        let stations=horizon_client::stations::StationRecipes::load(&root,&items);
        let enchantments=horizon_client::enchant::Enchantments::default();
        let names=horizon_client::creative::Names::default();
        let env=Env{items:&items,recipes:&recipes,brewing:&brewing,banners:&banners,
            station_recipes:&stations,enchantments:&enchantments,names:&names,trade:None,maps:None};
        let mut menu=Menu::new(Kind::Chest(2));
        menu.network_tool_slot=Some(8);menu.network_tool_main_limit=Some(9);
        menu.set_player_item(8,Some(ItemStack::new("ae2:network_tool",1)));
        // Chest(2) has 18 main slots, 27 main-inventory slots, then hotbar.
        menu.click(&env,53,0,Input::Pickup).unwrap();
        assert!(menu.carried.is_none());assert!(menu.player_item(8).is_some());
        menu.creative=true;menu.click(&env,53,2,Input::Clone).unwrap();
        assert_eq!(menu.carried.as_ref().map(|s|s.item.as_ref()),Some("ae2:network_tool"));
        assert!(menu.player_item(8).is_some());menu.carried=None;menu.creative=false;
        menu.click(&env,9,8,Input::Swap).unwrap();
        assert!(menu.player_item(8).is_none());
        assert_eq!(menu.slot(9).map(|s|s.item.as_ref()),Some("ae2:network_tool"));
        menu.set_slot(&env,9,None);menu.carried=Some(ItemStack::new("minecraft:diamond",3));
        menu.click(&env,0,0,Input::Pickup).unwrap();
        assert!(menu.slot(0).is_none());assert_eq!(menu.carried.as_ref().map(|s|s.count),Some(3));
        menu.click(&env,9,0,Input::Pickup).unwrap();
        assert_eq!(menu.slot(9).map(|s|s.count),Some(3));assert!(menu.carried.is_none());
        menu.network_tool_slot=Some(40);
        menu.set_player_item(40,Some(ItemStack::new("ae2:network_tool",1)));
        menu.set_player_item(0,Some(ItemStack::new("minecraft:cobblestone",2)));
        menu.set_slot(&env,10,None);menu.click(&env,10,0,Input::Swap).unwrap();
        assert!(menu.slot(10).is_none());assert_eq!(menu.player_item(0).map(|s|s.count),Some(2));
    }
    #[test]
    fn hand_menu_dispatch_skips_non_menu_blocks_and_keeps_real_menus() {
        use horizon_client::ae2_world::MachineKind as K;
        for kind in [K::QuantumRing,K::EnergyCell,K::Controller,K::Charger] {assert!(!ae2_hand_menu_kind(kind));}
        for kind in [K::QuantumLink,K::Chest,K::Drive,K::SkyStoneChest,K::SmoothSkyStoneChest] {assert!(ae2_hand_menu_kind(kind));}
    }
    #[test]
    fn encoding_revisions_distinguish_refreshes_from_changed_recipe() {
        use horizon_client::ae2_world::{Machine,MachineKind};
        let original=Machine::new(MachineKind::PatternEncodingTerminal);
        let mut changed=original.clone();let mut recipe=super::super::ae2_encoding::EncodingState::default();recipe.inputs[0]=Some(ItemStack::new("minecraft:stone",1));recipe.save(&mut changed).unwrap();
        let mut blank=original.clone();blank.slots[0]=Some(ItemStack::new("ae2:blank_pattern",1));
        let history=std::collections::VecDeque::from([(4,Some(original.clone())),(5,Some(original.clone())),(6,Some(changed.clone()))]);
        assert!(encoding_revision_matches(&history,4,Some(&original)),"older unchanged views remain usable");
        assert!(encoding_revision_matches(&history,4,Some(&blank)),"physical blank insertion does not change the viewed recipe");
        assert!(!encoding_revision_matches(&history,4,Some(&changed)),"a queued click cannot edit another viewer's new recipe");
        assert!(encoding_revision_matches(&history,6,Some(&changed)));
        assert!(!encoding_revision_matches(&history,3,Some(&changed)),"expired revisions fail closed");
    }
    #[test]
    fn search_sort_and_craftable_filter_keep_the_right_item_keys() {
        let key = |id: &str| ae2_cell_core::ItemKey {
            item_id: id.into(),
            components: vec![],
        };
        let entries = vec![
            (key("minecraft:oak_planks"), 20),
            (key("minecraft:oak_log"), 5),
            (key("minecraft:stick"), 0),
        ];
        let craftable = vec![key("minecraft:oak_log"), key("minecraft:stick")];
        let names = horizon_client::creative::Names::default();
        let result = terminal_filter_entries(&entries, &craftable, 1, true, 2, "minecraft:", &names);
        assert_eq!(
            result,
            vec![(key("minecraft:oak_log"), 5), (key("minecraft:stick"), 0)]
        );
        let result = terminal_filter_entries(&entries, &craftable, 0, false, 1, "planks", &names);
        assert_eq!(result, vec![(key("minecraft:oak_planks"), 20)]);
        let mixed=vec![(key("minecraft:apple"),1),(key("ae2:zeta"),1),(key("ae2:alpha"),1)];
        let result=terminal_filter_entries(&mixed,&[],2,false,0,"",&names);
        assert_eq!(result,vec![(key("ae2:alpha"),1),(key("ae2:zeta"),1),(key("minecraft:apple"),1)]);
    }
}

// Source Settings.INSCRIBER_SEPARATE_SIDES and INSCRIBER_INPUT_CAPACITY.
fn inscriber_setting_click(m:&mut horizon_client::ae2_world::Machine, slot:usize, reverse:bool)->bool {
    if m.kind!=horizon_client::ae2_world::MachineKind::Inscriber{return false}
    let (key,values):(&str,&[&str])=match slot {
        324=>("auto_export",&["NO","YES"]),
        325=>("inscriber_separate_sides",&["NO","YES"]),
        326=>("inscriber_input_capacity",&["SIXTY_FOUR","ONE","FOUR"]),
        _=>return false,
    };
    let current=match m.config.get(&key.into()) {
        None=>values[0],
        Some(Tag::String(s))=>match s.to_string().ok().and_then(|s|values.iter().copied().find(|v|*v==s)) {Some(value)=>value,None=>return false},
        _=>return false,
    };
    let index=values.iter().position(|v|*v==current).unwrap();
    let next=values[(index+if reverse {values.len()-1}else{1})%values.len()];
    m.config.insert(key.into(),Tag::String(next.into()));true
}
impl Game<'_> {
    fn ae2_inscriber_setting(&mut self, pos:(i32,i32,i32), slot:usize, reverse:bool)->bool {
        let Some(horizon_client::containers::BlockEntity::Ae2(m))=self.world.block_entities.get_mut(&pos) else{return false};
        if !inscriber_setting_click(m,slot,reverse){return false}
        self.world.modified.insert((pos.0>>4,pos.2>>4));true
    }
}
#[cfg(test)]
mod inscriber_setting_tests {
    use super::*;
    #[test]
    fn physical_inscriber_settings_cycle_source_values_and_reject_corrupt_state() {
        use horizon_client::ae2_world::{Machine,MachineKind};
        let mut machine=Machine::new(MachineKind::Inscriber);
        assert!(inscriber_setting_click(&mut machine,324,false));
        assert_eq!(machine.config.get(&"auto_export".into()),Some(&Tag::String("YES".into())));
        assert!(inscriber_setting_click(&mut machine,325,false));
        assert_eq!(machine.config.get(&"inscriber_separate_sides".into()),Some(&Tag::String("YES".into())));
        assert!(inscriber_setting_click(&mut machine,326,false));
        assert_eq!(machine.config.get(&"inscriber_input_capacity".into()),Some(&Tag::String("ONE".into())));
        assert!(inscriber_setting_click(&mut machine,326,true));
        assert_eq!(machine.config.get(&"inscriber_input_capacity".into()),Some(&Tag::String("SIXTY_FOUR".into())));
        machine.config.insert("inscriber_input_capacity".into(),Tag::Int(999));
        assert!(!inscriber_setting_click(&mut machine,326,false));
        assert_eq!(machine.config.get(&"inscriber_input_capacity".into()),Some(&Tag::Int(999)));
        assert!(!inscriber_setting_click(&mut Machine::new(MachineKind::Drive),325,false));
    }
}
