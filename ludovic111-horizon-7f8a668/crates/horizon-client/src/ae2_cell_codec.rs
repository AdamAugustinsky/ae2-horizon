//! Validated physical AE2 cell codec shared by game logic and rendering.
//! Derived from AE2 19.2.18 BasicCellInventory (LGPL-3.0-or-later).
use crate::{items::ItemStack, save};
use ae2_cell_core::{Cell, CellConfig, Channel, ItemKey, Tier};
use horizon_world::nbt::{self, Compound, Limits, Tag};
use std::collections::BTreeSet;
const COMPONENT: &str = "minecraft:custom_data";
const CELL_DATA: &str = "ae2_cell";
const FLUID_CELL_DATA: &str = "ae2_fluid_cell";
pub fn key(stack: &ItemStack) -> Result<ItemKey, String> {
    let prototype = ItemStack {
        count: 1,
        ..stack.clone()
    };
    let components = nbt::write_network(&save::item_to_nbt(&prototype), Limits::default())
        .map_err(|e| e.to_string())?;
    Ok(ItemKey {
        item_id: stack.item.to_string(),
        components,
    })
}

pub fn prototype(key: &ItemKey) -> Result<ItemStack, String> {
    let (tag, used) =
        nbt::read_network(&key.components, Limits::default()).map_err(|e| e.to_string())?;
    if used != key.components.len() {
        return Err("trailing item data".into());
    }
    let item = save::item_from_nbt(&tag).ok_or("invalid stored item")?;
    if &*item.item != key.item_id || item.count != 1 {
        return Err("stored item identity mismatch".into());
    }
    Ok(item)
}

pub fn is_cell(id: &str) -> bool {
    id.starts_with("ae2:item_storage_cell_")
        || id.starts_with("ae2:fluid_storage_cell_")
        || id.starts_with("ae2:portable_item_cell_")
        || id.starts_with("ae2:portable_fluid_cell_")
}
fn cell_config(id: &str) -> Option<CellConfig> {
    match id {
        "ae2:matter_cannon" => return Some(CellConfig::matter_cannon()),
        "ae2:color_applicator" => return Some(CellConfig::color_applicator()),
        _ => {}
    }
    let portable = id.starts_with("ae2:portable_item_cell_");
    let tier = match id
        .strip_prefix("ae2:item_storage_cell_")
        .or_else(|| id.strip_prefix("ae2:portable_item_cell_"))?
    {
        "1k" => Tier::K1,
        "4k" => Tier::K4,
        "16k" => Tier::K16,
        "64k" => Tier::K64,
        "256k" => Tier::K256,
        _ => return None,
    };
    Some(if portable {
        CellConfig::portable_item(tier)
    } else {
        CellConfig::item(tier)
    })
}
/// Check a nested cell without recursively decoding its item contents.
pub fn nested_cell_empty(stack: &ItemStack) -> bool {
    if stack.count != 1 {
        return false;
    }
    if stack.item.starts_with("ae2:fluid_storage_cell_")
        || stack.item.starts_with("ae2:portable_fluid_cell_")
    {
        return read_fluid_cell(stack).is_ok_and(|cell| cell.entries().next().is_none());
    }
    let Some(mut config) = cell_config(&stack.item) else {
        return false;
    };
    if apply_cell_settings(stack, &mut config).is_err() {
        return false;
    }
    match stack.data().and_then(|d| d.other(COMPONENT)) {
        None => true,
        Some(Tag::Compound(data)) => match data.get(&CELL_DATA.into()) {
            None => true,
            Some(tag) => {
                tag.get("version") == Some(&Tag::Int(1))
                    && matches!(tag.get("entries"), Some(Tag::List(entries)) if entries.is_empty())
            }
        },
        _ => false,
    }
}
pub fn read_cell(stack: &ItemStack) -> Result<Cell, String> {
    if cell_config(&stack.item).is_none() || stack.count != 1 {
        return Err("put exactly one ae2:item_storage_cell_1k in your offhand".into());
    }
    let mut config = cell_config(&stack.item).ok_or("unsupported cell tier")?;
    apply_cell_settings(stack, &mut config)?;
    let Some(tag) = stack
        .data()
        .and_then(|d| d.other(COMPONENT))
        .and_then(|t| t.get(CELL_DATA))
    else {
        return Ok(Cell::new(config));
    };
    if tag.get("version") != Some(&Tag::Int(1)) {
        return Err("unsupported cell data version".into());
    }
    let Some(Tag::List(entries)) = tag.get("entries") else {
        return Err("invalid cell entries".into());
    };
    if entries.len() > Cell::MAX_TYPES {
        return Err("cell exceeds its type limit".into());
    }
    let mut seen = BTreeSet::new();
    let mut restored = Vec::new();
    for entry in entries {
        let item_tag = entry.get("item").ok_or("missing cell item")?;
        // ItemStack's general decoder repairs zero/negative counts. A cell's
        // prototype schema is stricter: it must already contain exactly one.
        if item_tag.get("count") != Some(&Tag::Int(1)) {
            return Err("invalid stored item prototype count".into());
        }
        let item = save::item_from_nbt(item_tag).ok_or("invalid cell item")?;
        let Some(Tag::Long(amount)) = entry.get("amount") else {
            return Err("invalid stored count".into());
        };
        if *amount <= 0 || item.count != 1 || (!can_store_item(&item)) {
            return Err("invalid count or nested cell".into());
        }
        let item_key = key(&item)?;
        if !seen.insert(item_key.clone()) {
            return Err("duplicate stored item key".into());
        }
        restored.push((item_key, *amount as u64));
    }
    Cell::from_entries(config, restored).map_err(|e| format!("invalid cell capacity: {e:?}"))
}

pub fn write_cell(stack: &mut ItemStack, cell: &Cell) -> Result<(), String> {
    let entries = cell
        .entries()
        .map(|(key, amount)| {
            Ok(Tag::compound([
                ("item", save::item_to_nbt(&prototype(key)?)),
                ("amount", Tag::Long(*amount as i64)),
            ]))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut data = match stack.data().and_then(|d| d.other(COMPONENT)) {
        Some(Tag::Compound(map)) => map.clone(),
        None => Default::default(),
        _ => return Err("invalid cell custom data".into()),
    };
    data.insert(
        CELL_DATA.into(),
        Tag::compound([("version", Tag::Int(1)), ("entries", Tag::List(entries))]),
    );
    stack
        .data_mut()
        .set_other(COMPONENT, Some(Tag::Compound(data)));
    Ok(())
}

fn fluid_config(id: &str) -> Option<CellConfig> {
    let (suffix, portable) = if let Some(s) = id.strip_prefix("ae2:fluid_storage_cell_") {
        (s, false)
    } else {
        (id.strip_prefix("ae2:portable_fluid_cell_")?, true)
    };
    let tier = match suffix {
        "1k" => Tier::K1,
        "4k" => Tier::K4,
        "16k" => Tier::K16,
        "64k" => Tier::K64,
        "256k" => Tier::K256,
        _ => return None,
    };
    Some(if portable {
        CellConfig::portable_fluid(tier)
    } else {
        CellConfig::fluid(tier)
    })
}
pub fn bucket_fluid(id: &str) -> Option<&'static str> {
    match id {
        "minecraft:water_bucket" => Some("minecraft:water"),
        "minecraft:lava_bucket" => Some("minecraft:lava"),
        _ => None,
    }
}
pub fn fluid_bucket(id: &str) -> Option<&'static str> {
    match id {
        "minecraft:water" => Some("minecraft:water_bucket"),
        "minecraft:lava" => Some("minecraft:lava_bucket"),
        _ => None,
    }
}
pub fn fluid_key(id: &str) -> ItemKey {
    ItemKey {
        item_id: id.to_owned(),
        components: Vec::new(),
    }
}
pub fn valid_fluid_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        && path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-/".contains(&c))
        && id != "minecraft:empty"
}
/// Read the cell item itself. Unknown fluid IDs/components remain preserved;
/// bucket interaction rejects them unless the host has an exact mapping.
pub fn read_fluid_cell(stack: &ItemStack) -> Result<Cell, String> {
    if stack.count != 1 {
        return Err("hold exactly one fluid storage cell in your offhand".into());
    }
    let mut config =
        fluid_config(&stack.item).ok_or("offhand is not a supported fluid storage cell")?;
    apply_cell_settings(stack, &mut config)?;
    let custom = match stack.data().and_then(|d| d.other(COMPONENT)) {
        None => return Ok(Cell::new(config)),
        Some(Tag::Compound(map)) => map,
        Some(_) => return Err("invalid cell custom data".into()),
    };
    let Some(tag) = custom.get(&FLUID_CELL_DATA.into()) else {
        return Ok(Cell::new(config));
    };
    if tag.get("version") != Some(&Tag::Int(1)) {
        return Err("unsupported fluid cell version".into());
    }
    let Some(Tag::List(entries)) = tag.get("entries") else {
        return Err("invalid fluid entries".into());
    };
    if entries.len() > config.max_types() {
        return Err("fluid cell exceeds type limit".into());
    }
    let mut restored = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for entry in entries {
        let Some(Tag::String(id)) = entry.get("fluid") else {
            return Err("invalid fluid id".into());
        };
        let id = id.to_string().map_err(|_| "invalid fluid id")?;
        if !valid_fluid_id(&id) {
            return Err("invalid fluid id".into());
        }
        let Some(Tag::ByteArray(components)) = entry.get("components") else {
            return Err("invalid fluid components".into());
        };
        let Some(Tag::Long(amount)) = entry.get("amount") else {
            return Err("invalid fluid amount".into());
        };
        if *amount <= 0 {
            return Err("invalid fluid amount".into());
        }
        let key = ItemKey {
            item_id: id,
            components: components.clone(),
        };
        if !seen.insert(key.clone()) {
            return Err("duplicate fluid key".into());
        }
        restored.push((key, *amount as u64));
    }
    Cell::from_entries(config, restored).map_err(|e| format!("invalid fluid cell capacity: {e:?}"))
}
/// Store validated fluid contents in the physical ItemStack. Other custom data
/// is preserved; no implicit conversion from Java AE2's native NBT is claimed.
pub fn write_fluid_cell(stack: &mut ItemStack, cell: &Cell) -> Result<(), String> {
    let expected = fluid_config(&stack.item).ok_or("unsupported fluid storage cell")?;
    if stack.count != 1
        || cell.config().channel != Channel::Fluid
        || cell.config().tier != expected.tier
        || cell.config().portable != expected.portable
    {
        return Err("fluid cell type mismatch".into());
    }
    let mut entries = Vec::new();
    for (key, amount) in cell.entries() {
        if !valid_fluid_id(&key.item_id) || *amount > i64::MAX as u64 {
            return Err("invalid fluid entry".into());
        }
        entries.push(Tag::compound([
            ("fluid", Tag::String(key.item_id.as_str().into())),
            ("components", Tag::ByteArray(key.components.clone())),
            ("amount", Tag::Long(*amount as i64)),
        ]));
    }
    let mut data = match stack.data().and_then(|d| d.other(COMPONENT)) {
        Some(Tag::Compound(m)) => m.clone(),
        None => Default::default(),
        _ => return Err("invalid cell custom data".into()),
    };
    data.insert(
        FLUID_CELL_DATA.into(),
        Tag::compound([("version", Tag::Int(1)), ("entries", Tag::List(entries))]),
    );
    stack
        .data_mut()
        .set_other(COMPONENT, Some(Tag::Compound(data)));
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreativeCellKey {
    Item(ae2_cell_core::ItemKey),
    Fluid(ae2_cell_core::ItemKey),
}
/// Physical View Cell configuration (AE2 ViewCellItem / CellConfig).
#[derive(Clone, Debug)]
pub struct ViewCellSettings {
    pub filters: Vec<CreativeCellKey>,
    pub fuzzy: bool,
    pub inverted: bool,
    pub fuzzy_mode: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCellScope { All, ItemsOnly }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewCellMatch { pub configured: bool, pub inverted: bool, pub matches: bool }
/// Preserve the source 63-slot ghost inventory, with a legacy dense-list
/// fallback. Entries remain full item/fluid tags; Slot is metadata only.
pub fn decode_view_filter_slots(entries: &[Tag]) -> Result<Vec<Option<Tag>>, String> {
    let mut slots = vec![None; 63];
    for (legacy, entry) in entries.iter().enumerate() {
        let index = match entry.get("Slot") {
            None => legacy,
            Some(Tag::Int(n)) if *n >= 0 => *n as usize,
            _ => return Err("invalid view-cell filter slot".into()),
        };
        let target = slots.get_mut(index).ok_or("view-cell filter slot out of range")?;
        if target.is_some() { return Err("duplicate view-cell filter slot".into()); }
        if !matches!(entry,Tag::Compound(_)) { return Err("invalid view-cell filter".into()); }
        *target = Some(entry.clone());
    }
    Ok(slots)
}
pub fn encode_view_filter_slots(slots: &[Option<Tag>]) -> Result<Vec<Tag>, String> {
    if slots.len() != 63 { return Err("invalid view-cell filter inventory size".into()); }
    slots.iter().enumerate().filter_map(|(index,entry)| entry.as_ref().map(|entry|(index,entry)))
        .map(|(index,entry)| {
            let Tag::Compound(mut data)=entry.clone() else {return Err("invalid view-cell filter".into())};
            data.insert("Slot".into(),Tag::Int(index as i32));
            Ok(Tag::Compound(data))
        }).collect()
}
impl ViewCellSettings {
    pub fn evaluate(&self, key: &ItemKey, fluid: bool, scope: ViewCellScope,
        fuzzy_item: impl Fn(&ItemKey, &ItemKey, &str) -> bool) -> ViewCellMatch {
        let eligible = |entry: &CreativeCellKey| scope == ViewCellScope::All || matches!(entry, CreativeCellKey::Item(_));
        let configured = self.filters.iter().any(&eligible);
        let matches = self.filters.iter().filter(|entry| eligible(entry)).any(|entry| match entry {
            CreativeCellKey::Item(filter) if !fluid => if self.fuzzy { fuzzy_item(filter, key, &self.fuzzy_mode) } else { filter == key },
            CreativeCellKey::Fluid(filter) if fluid => if self.fuzzy { filter.item_id == key.item_id } else { filter == key },
            _ => false,
        });
        ViewCellMatch { configured, inverted:self.inverted, matches }
    }
}

pub fn read_view_cell(stack: &ItemStack) -> Result<ViewCellSettings, String> {
    if stack.item.as_ref() != "ae2:view_cell" || stack.count != 1 { return Err("not a singular view cell".into()); }
    let upgrades = crate::ae2_upgrade_inventory::Inventory::read(stack)?;
    let fuzzy = upgrades.slots.iter().flatten().any(|card| card.item.as_ref() == "ae2:fuzzy_card");
    let inverted = upgrades.slots.iter().flatten().any(|card| card.item.as_ref() == "ae2:inverter_card");
    let settings = cell_settings(stack)?;
    let fuzzy_mode = match settings.get(&"fuzzy_mode".into()) {
        None => "ignore_all".to_owned(),
        Some(Tag::String(mode)) => mode.to_string().map_err(|_| "invalid view-cell fuzzy mode")?,
        _ => return Err("invalid view-cell fuzzy mode".into()),
    };
    if !["ignore_all","percent_99","percent_75","percent_50","percent_25"].contains(&fuzzy_mode.as_str()) {
        return Err("invalid view-cell fuzzy mode".into());
    }
    let Some(Tag::List(entries)) = settings.get(&"filters".into()) else { return Err("invalid view-cell filters".into()) };
    let slots = decode_view_filter_slots(entries)?;
    let mut filters = Vec::new();
    for entry in slots.iter().flatten() {
        let parsed = match (entry.get("item"), entry.get("fluid"), entry.get("components")) {
            (Some(item), None, None) if item.get("count") == Some(&Tag::Int(1)) => {
                let item = save::item_from_nbt(item).ok_or("invalid view-cell item")?;
                CreativeCellKey::Item(key(&item)?)
            }
            (None, Some(Tag::String(id)), Some(Tag::ByteArray(components))) => {
                let id=id.to_string().map_err(|_| "invalid view-cell fluid")?;
                if !valid_fluid_id(&id) { return Err("invalid view-cell fluid".into()); }
                CreativeCellKey::Fluid(ItemKey { item_id:id, components:components.clone() })
            }
            _ => return Err("invalid view-cell filter".into()),
        };
        filters.push(parsed);
    }
    Ok(ViewCellSettings { filters, fuzzy, inverted, fuzzy_mode })
}
pub fn creative_cell_keys(stack: &ItemStack) -> Result<Vec<CreativeCellKey>, String> {
    if stack.item.as_ref() != "ae2:creative_storage_cell" || stack.count != 1 {
        return Err("not a creative storage cell".into());
    }
    let settings = cell_settings(stack)?;
    let Some(Tag::List(cards)) = settings.get(&"upgrades".into()) else {
        return Err("invalid creative cell cards".into());
    };
    if !cards.is_empty() {
        return Err("creative storage cell has no upgrade slots".into());
    }
    let Some(Tag::List(filters)) = settings.get(&"filters".into()) else {
        return Err("invalid creative cell filters".into());
    };
    if filters.len() > 63 {
        return Err("too many creative cell keys".into());
    }
    let mut keys = Vec::new();
    for filter in filters {
        let key = if let Some(tag) = filter.get("item") {
            if tag.get("count") != Some(&Tag::Int(1)) {
                return Err("invalid creative item key".into());
            }
            let item = save::item_from_nbt(tag).ok_or("invalid creative item key")?;
            CreativeCellKey::Item(key(&item)?)
        } else if let (Some(Tag::String(id)), Some(Tag::ByteArray(components))) =
            (filter.get("fluid"), filter.get("components"))
        {
            let id = id.to_string().map_err(|_| "invalid creative fluid key")?;
            if !valid_fluid_id(&id) {
                return Err("invalid creative fluid key".into());
            }
            CreativeCellKey::Fluid(ae2_cell_core::ItemKey {
                item_id: id,
                components: components.clone(),
            })
        } else {
            return Err("invalid creative cell key".into());
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}
pub fn apply_cell_settings(
    stack: &ItemStack,
    config: &mut ae2_cell_core::CellConfig,
) -> Result<(), String> {
    use ae2_cell_core::{Channel, Filter, FilterMode, ItemKey};
    let Some(custom) = stack.data().and_then(|d| d.other("minecraft:custom_data")) else {
        return Ok(());
    };
    let Tag::Compound(map) = custom else {
        return Err("invalid cell custom data".into());
    };
    let Some(settings) = map.get(&"ae2_cell_settings".into()) else {
        return Ok(());
    };
    let Some(Tag::List(cards)) = settings.get("upgrades") else {
        return Err("invalid cell upgrades".into());
    };
    let max = match config.tool {
        Some(ae2_cell_core::ToolCell::ColorApplicator) => 2,
        Some(ae2_cell_core::ToolCell::MatterCannon) => 4,
        None if config.channel == Channel::Item => 4,
        None => 3,
    };
    crate::ae2_upgrade_inventory::decode(cards,max)?;
    if cards.len() > max {
        return Err("too many cell upgrades".into());
    };
    let fuzzy_mode = match settings.get("fuzzy_mode") {
        None => "ignore_all".to_owned(),
        Some(Tag::String(s)) => s.to_string().map_err(|_| "invalid cell fuzzy mode")?,
        _ => return Err("invalid cell fuzzy mode".into()),
    };
    config.fuzzy_mode = match fuzzy_mode.as_str() {
        "ignore_all" => ae2_cell_core::FuzzyMode::IgnoreAll,
        "percent_99" => ae2_cell_core::FuzzyMode::Percent99,
        "percent_75" => ae2_cell_core::FuzzyMode::Percent75,
        "percent_50" => ae2_cell_core::FuzzyMode::Percent50,
        "percent_25" => ae2_cell_core::FuzzyMode::Percent25,
        _ => return Err("invalid cell fuzzy mode".into()),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut energy_cards = 0u8;
    for card in cards {
        if card.get("count") != Some(&Tag::Int(1)) {
            return Err("invalid cell card count".into());
        };
        let card = save::item_from_nbt(card).ok_or("invalid cell card")?;
        if card.item.as_ref() == "ae2:energy_card" && (config.portable || config.tool.is_some()) {
            energy_cards += 1;
            if energy_cards > 2 {
                return Err("too many energy cards".into());
            }
            continue;
        }
        if config.tool == Some(ae2_cell_core::ToolCell::MatterCannon)
            && card.item.as_ref() == "ae2:speed_card"
        {
            continue;
        }
        if !seen.insert(card.item.to_string()) {
            return Err("duplicate cell card".into());
        };
        if matches!(config.tool, Some(ae2_cell_core::ToolCell::ColorApplicator))
            && !matches!(
                card.item.as_ref(),
                "ae2:equal_distribution_card" | "ae2:void_card"
            )
        {
            return Err("unsupported color applicator card".into());
        }
        if matches!(config.tool, Some(ae2_cell_core::ToolCell::MatterCannon))
            && card.item.as_ref() == "ae2:equal_distribution_card"
        {
            return Err("unsupported matter cannon card".into());
        }
        match card.item.as_ref() {
            "ae2:inverter_card" => {}
            "ae2:equal_distribution_card" => config.equal_distribution = true,
            "ae2:void_card" => config.void_excess = true,
            "ae2:fuzzy_card" if config.channel == Channel::Item => config.fuzzy = true,
            _ => return Err("unsupported cell card".into()),
        }
    }
    let Some(Tag::List(filters)) = settings.get("filters") else {
        return Err("invalid cell filters".into());
    };
    if filters.len() > 63 {
        return Err("too many cell filters".into());
    };
    let mut keys = std::collections::BTreeSet::new();
    for filter in filters {
        let key = if config.channel == Channel::Item {
            let item = filter
                .get("item")
                .and_then(save::item_from_nbt)
                .ok_or("invalid cell item filter")?;
            key(&item)?
        } else {
            let Some(Tag::String(id)) = filter.get("fluid") else {
                return Err("invalid fluid filter".into());
            };
            let Some(Tag::ByteArray(components)) = filter.get("components") else {
                return Err("invalid fluid filter components".into());
            };
            ItemKey {
                item_id: id.to_string().map_err(|_| "invalid fluid id")?,
                components: components.clone(),
            }
        };
        keys.insert(key);
    }
    config.filter = Some(Filter {
        mode: if seen.contains("ae2:inverter_card") {
            FilterMode::Blacklist
        } else {
            FilterMode::Whitelist
        },
        keys,
    });
    Ok(())
}

pub fn cell_settings(stack: &ItemStack) -> Result<Compound, String> {
    let existing = match stack.data().and_then(|d| d.other("minecraft:custom_data")) {
        None => None,
        Some(Tag::Compound(m)) => m.get(&"ae2_cell_settings".into()),
        Some(_) => return Err("invalid cell custom data".into()),
    };
    match existing {
        None => Ok(
            match Tag::compound([
                ("upgrades", Tag::List(vec![])),
                ("filters", Tag::List(vec![])),
            ]) {
                Tag::Compound(m) => m,
                _ => unreachable!(),
            },
        ),
        Some(Tag::Compound(m)) => Ok(m.clone()),
        _ => Err("invalid cell settings".into()),
    }
}
pub fn can_store_item(stack: &ItemStack) -> bool {
    if stack.item.as_ref() == "ae2:creative_storage_cell" {
        return creative_cell_keys(stack).is_ok_and(|keys| keys.is_empty());
    }
    if !is_cell(&stack.item) {
        return true;
    }
    nested_cell_empty(stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae2_cell_core::Action;
    #[test]
    fn shared_item_codec_roundtrips_components_and_rejects_corruption() {
        let mut item = ItemStack::new("minecraft:diamond_sword", 1);
        item.damage = 7;
        item.data_mut().custom_name = Some("Physical tool".into());
        let key = key(&item).unwrap();
        let mut cell = Cell::new_1k();
        assert_eq!(cell.insert(&key, 1, Action::Modulate), 1);
        let mut holder = ItemStack::new("ae2:item_storage_cell_1k", 1);
        write_cell(&mut holder, &cell).unwrap();
        let restored = read_cell(&holder).unwrap();
        assert_eq!(
            prototype(restored.entries().next().unwrap().0).unwrap(),
            item
        );
        holder.data_mut().set_other(
            COMPONENT,
            Some(Tag::compound([(
                CELL_DATA,
                Tag::compound([("version", Tag::Int(99)), ("entries", Tag::List(vec![]))]),
            )])),
        );
        assert!(read_cell(&holder).is_err());
    }
    #[test]
    fn view_cell_validates_physical_cards_and_mixed_channel_scopes() {
        let mut cell=ItemStack::new("ae2:view_cell",1);
        let empty=read_view_cell(&cell).unwrap();
        let diamond=ItemStack::new("minecraft:diamond",1); let diamond_key=key(&diamond).unwrap();
        assert!(!empty.evaluate(&diamond_key,false,ViewCellScope::All,|_,_,_|false).configured);
        let card=|id| save::item_to_nbt(&ItemStack::new(id,1));
        let settings=Tag::compound([
            ("upgrades",Tag::List(vec![card("ae2:fuzzy_card"),card("ae2:inverter_card")])),
            ("fuzzy_mode",Tag::String("percent_50".into())),
            ("filters",Tag::List(vec![Tag::compound([("item",save::item_to_nbt(&diamond))]),
                Tag::compound([("fluid",Tag::String("minecraft:water".into())),("components",Tag::ByteArray(vec![]))])]))]);
        cell.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([("ae2_cell_settings",settings)])));
        let view=read_view_cell(&cell).unwrap();
        let matched=view.evaluate(&diamond_key,false,ViewCellScope::All,|a,b,m|a==b && m=="percent_50");
        assert!(matched.configured && matched.inverted && matched.matches);
        let water=fluid_key("minecraft:water");
        assert!(view.evaluate(&water,true,ViewCellScope::All,|_,_,_|false).matches);
        assert!(!view.evaluate(&water,false,ViewCellScope::All,|_,_,_|false).matches);
        let only_fluid=ViewCellSettings {filters:vec![CreativeCellKey::Fluid(water)],..view};
        assert!(only_fluid.evaluate(&diamond_key,false,ViewCellScope::All,|_,_,_|false).configured);
        assert!(!only_fluid.evaluate(&diamond_key,false,ViewCellScope::ItemsOnly,|_,_,_|false).configured);
        let mut settings=cell_settings(&cell).unwrap();settings.insert("upgrades".into(),Tag::List(vec![card("ae2:speed_card")]));
        cell.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([("ae2_cell_settings",Tag::Compound(settings))])));
        assert!(read_view_cell(&cell).is_err());
    }

    #[test]
    fn view_filter_slots_preserve_holes_and_reject_collisions() {
        let mut slots=vec![None;63];
        slots[17]=Some(Tag::compound([("fluid",Tag::String("minecraft:water".into())),("components",Tag::ByteArray(vec![]))]));
        let entries=encode_view_filter_slots(&slots).unwrap();
        let restored=decode_view_filter_slots(&entries).unwrap();
        assert!(restored[..17].iter().all(Option::is_none));
        assert_eq!(restored[17].as_ref().and_then(|tag|tag.get("fluid")),Some(&Tag::String("minecraft:water".into())));
        assert!(decode_view_filter_slots(&[entries[0].clone(),entries[0].clone()]).is_err());
    }

}
