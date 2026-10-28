// Interface Memory Card support is kept inside ae2_memory so it can use the
// same physical reconciliation helpers without another shared game.rs hook.
mod interface_memory {
    use super::*;
    use super::super::ae2_interface::{interface_config, interface_fluid_config, mark_card_removed, wake_interface};

    const ITEM_CONFIG: &str = "ae2_interface_config";
    const FLUID_CONFIG: &str = "ae2_interface_fluid_config";
    const FUZZY: &str = "ae2_fuzzy_mode";
    const FUZZY_VALUES: &[&str] = &["ignore_all", "percent_99", "percent_75", "percent_50", "percent_25"];
    const BLOCK_SOURCE: &str = "ae2:interface";
    const PART_SOURCE: &str = "ae2:cable_interface";

    #[derive(Clone, Debug)]
    pub(in crate::game) struct SavedInterface {
        pub fuzzy: Option<String>,
        pub priority: Option<i32>,
        /// The two native lists jointly represent Java's one config component.
        pub ghosts: Option<(Tag,Tag)>,
        /// None means absent; Some(None) explicitly clears installed upgrades.
        pub desired: Option<Option<ItemStack>>,
        pub name: Option<String>,
        /// Parts export their own description; targets compare the block one.
        pub part_source: bool,
    }

    fn validate_upgrade(machine: &Machine) -> Result<(), String> {
        if machine.kind != MachineKind::Interface || machine.upgrades.len() != 1 {
            return Err("invalid Interface upgrade inventory".into());
        }
        if machine.upgrades.iter().flatten().any(|s| s.count != 1 || !matches!(s.item.as_ref(), "ae2:crafting_card" | "ae2:fuzzy_card")) {
            return Err("invalid Interface upgrade".into());
        }
        Ok(())
    }
    fn validate_config(machine: &Machine) -> Result<(), String> {
        let items = interface_config(machine)?;
        let fluids = interface_fluid_config(machine)?;
        if items.iter().zip(fluids.iter()).any(|(a, b)| a.is_some() && b.is_some()) {
            return Err("conflicting Interface ghost configuration".into());
        }
        Ok(())
    }
    fn snapshot(machine: &Machine, name: Option<&str>) -> Result<SavedInterface, String> {
        validate_upgrade(machine)?;
        validate_config(machine)?;
        let fuzzy = setting_value(machine, FUZZY, "ignore_all", FUZZY_VALUES)?;
        let priority = match machine.config.get(&"priority".into()) {
            None => 0,
            Some(Tag::Int(value)) => *value,
            _ => return Err("invalid Interface priority".into()),
        };
        Ok(SavedInterface {
            fuzzy:Some(fuzzy),
            priority:Some(priority),
            ghosts:Some((machine.config.get(&ITEM_CONFIG.into()).cloned().unwrap_or_else(|| Tag::List(Vec::new())),
                         machine.config.get(&FLUID_CONFIG.into()).cloned().unwrap_or_else(|| Tag::List(Vec::new())))),
            desired:Some(machine.upgrades[0].clone()),
            name: name.map(str::to_owned),
            part_source: false,
        })
    }
    fn validate_saved(saved: &SavedInterface) -> Result<(), String> {
        if saved.fuzzy.as_ref().is_some_and(|v|!FUZZY_VALUES.contains(&v.as_str())) { return Err("invalid fuzzy mode".into()); }
        let mut machine = Machine::new(MachineKind::Interface);
        machine.upgrades[0] = saved.desired.clone().flatten();
        if let Some((items,fluids))=&saved.ghosts {
            machine.config.insert(ITEM_CONFIG.into(),items.clone());
            machine.config.insert(FLUID_CONFIG.into(),fluids.clone());
        }
        validate_upgrade(&machine)?;
        validate_config(&machine)
    }
    fn save_interface_source(card: &mut ItemStack, machine: &Machine, name: Option<&str>, source: &str) -> Result<(), String> {
        if card.item.as_ref() != "ae2:memory_card" || card.count != 1 { return Err("invalid card".into()); }
        let saved = snapshot(machine, name)?;
        let mut data = match card.data().and_then(|d| d.other("minecraft:custom_data")) {
            None => Compound::new(), Some(Tag::Compound(c)) => c.clone(), _ => return Err("invalid card custom data".into()),
        };
        let mut body = Compound::new();
        body.insert("source".into(), Tag::String(source.into()));
        body.insert("settings".into(), Tag::compound([(FUZZY, Tag::String(saved.fuzzy.as_deref().ok_or("missing snapshot fuzzy")?.into()))]));
        body.insert("priority".into(), Tag::Int(saved.priority.ok_or("missing snapshot priority")?));
        let (items,fluids)=saved.ghosts.ok_or("missing snapshot configuration")?;
        body.insert("config_items".into(), items);
        body.insert("config_fluids".into(), fluids);
        body.insert("upgrades".into(), Tag::List(saved.desired.ok_or("missing snapshot upgrades")?.iter().map(save::item_to_nbt).collect()));
        if let Some(name) = saved.name { body.insert("name".into(), Tag::String(name.as_str().into())); }
        data.remove(&P2P.into());data.insert(KEY.into(), Tag::Compound(body));
        card.data_mut().set_other("minecraft:custom_data", Some(Tag::Compound(data)));
        Ok(())
    }
    pub(in crate::game) fn save_interface(card: &mut ItemStack, machine: &Machine, name: Option<&str>) -> Result<(), String> {
        save_interface_source(card, machine, name, BLOCK_SOURCE)
    }
    pub(in crate::game) fn save_interface_part(card: &mut ItemStack, machine: &Machine, name: Option<&str>) -> Result<(), String> {
        save_interface_source(card, machine, name, PART_SOURCE)
    }
    pub(in crate::game) fn read_interface(card: &ItemStack) -> Result<Option<SavedInterface>, String> {
        if card.item.as_ref() != "ae2:memory_card" || card.count != 1 { return Err("invalid card".into()); }
        let Some(data) = card.data().and_then(|d| d.other("minecraft:custom_data")) else { return Ok(None); };
        let Some(body) = data.get(KEY) else { return Ok(None); };
        let part_source = if body.get("source") == Some(&Tag::String(BLOCK_SOURCE.into())) { false }
            else if body.get("source") == Some(&Tag::String(PART_SOURCE.into())) { true }
            else { return Ok(None); };
        let fuzzy = match body.get("settings") {
            None => None,
            Some(Tag::Compound(settings)) => match settings.get(&FUZZY.into()) {
                None => None,
                Some(Tag::String(s)) => {let value=s.to_string().map_err(|_|"invalid fuzzy text")?;FUZZY_VALUES.contains(&value.as_str()).then_some(value)},
                _ => return Err("invalid fuzzy mode".into()),
            },
            _ => return Err("invalid settings".into()),
        };
        let priority = match body.get("priority") {None=>None,Some(Tag::Int(v))=>Some(*v),_=>return Err("invalid priority".into())};
        let ghosts = match (body.get("config_items"),body.get("config_fluids")) {
            (None,None)=>None,(Some(items),Some(fluids))=>Some((items.clone(),fluids.clone())),
            _=>return Err("incomplete native Interface config component".into()),
        };
        let desired = match body.get("upgrades") {
            None=>None,
            Some(Tag::List(upgrades))=>{
                if upgrades.len()>1{return Err("too many Interface upgrades".into())}
                let upgrade=if let Some(entry)=upgrades.first(){
                    if entry.get("count")!=Some(&Tag::Int(1)){return Err("invalid upgrade count".into())}
                    Some(save::item_from_nbt(entry).ok_or("invalid upgrade")?)
                }else{None};Some(upgrade)
            },
            _=>return Err("invalid upgrades".into()),
        };
        let name = match body.get("name") { None => None, Some(Tag::String(s)) => Some(s.to_string().map_err(|_| "invalid name")?), _ => return Err("invalid name".into()) };
        let saved = SavedInterface { fuzzy, priority, ghosts, desired, name, part_source };
        validate_saved(&saved)?;
        Ok(Some(saved))
    }
    /// Generic cross-kind import reads the priority component independently;
    /// it must not parse or reconstruct Interface ghost rows or upgrades.
    pub(in crate::game) fn interface_priority_from_card(card: &ItemStack) -> Result<Option<i32>, String> {
        if card.item.as_ref() != "ae2:memory_card" || card.count != 1 { return Err("invalid card".into()); }
        let Some(data) = card.data().and_then(|d| d.other("minecraft:custom_data")) else { return Ok(None); };
        if !matches!(data, Tag::Compound(_)) { return Err("invalid card custom data".into()); }
        let Some(body) = data.get(KEY) else { return Ok(None); };
        if body.get("source") != Some(&Tag::String(BLOCK_SOURCE.into()))
            && body.get("source") != Some(&Tag::String(PART_SOURCE.into())) { return Ok(None); }
        match body.get("priority") {
            None => Ok(None),
            Some(Tag::Int(priority)) => Ok(Some(*priority)),
            _ => Err("invalid Interface priority".into()),
        }
    }
    /// InterfaceLogic.setPriority only stores the priority and saves the host.
    /// In particular, a generic provider card carries no exported upgrades or
    /// config inventory and must not cancel jobs or modify the target's rows.
    pub(in crate::game) fn apply_interface_priority(machine: &mut Machine, priority: i32) -> Result<(), String> {
        if machine.kind != MachineKind::Interface { return Err("invalid Interface target".into()); }
        machine.config.insert("priority".into(), Tag::Int(priority));
        Ok(())
    }
    /// Existing supported machine cards have no Interface-compatible upgrades
    /// or matching settings, priority, or ghost-inventory components.
    pub(in crate::game) fn partial_interface(machine: &Machine) -> Result<SavedInterface, String> {
        let mut saved = snapshot(machine, None)?;
        saved.fuzzy=None;saved.priority=None;saved.ghosts=None;saved.desired=Some(None);
        Ok(saved)
    }
    pub(in crate::game) fn interface_has_components(saved: &SavedInterface) -> bool {
        saved.fuzzy.is_some() || saved.priority.is_some() || saved.ghosts.is_some() || saved.desired.is_some()
    }
    pub(in crate::game) fn interface_to_inscriber(machine: &Machine) -> Result<SavedInscriber, String> {
        vibration_to_inscriber(machine, &SavedVibration { desired: Vec::new(), name: None })
    }
    pub(in crate::game) fn interface_to_io(machine: &Machine) -> Result<SavedIo, String> {
        vibration_to_io(machine, &SavedVibration { desired: Vec::new(), name: None })
    }
    pub(in crate::game) fn interface_to_vibration() -> SavedVibration {
        SavedVibration { desired: Vec::new(), name: None }
    }
    pub(in crate::game) fn apply_interface(
        machine: &mut Machine, player: &mut Vec<Option<ItemStack>>,
        pocket: &mut Option<horizon_client::ae2_network_tool::Inventory>, saved: &SavedInterface,
        creative: bool, same: bool,
    ) -> Result<PhysicalChanges, String> {
        if player.len() != 36 { return Err("invalid player inventory".into()); }
        validate_upgrade(machine)?;validate_config(machine)?;validate_saved(saved)?;
        let mut staged=machine.clone();let mut inventory=player.clone();let mut tool=pocket.clone();
        let mut changes=PhysicalChanges {drops:Vec::new(),changed_tracked:Vec::new(),replaced_tracked:Vec::new(),missing:0};
        let mut upgrade_callback=false;
        if let Some(desired)=saved.desired.as_ref() {
            if creative {
                // Source setItemDirect invokes the no-Crafting-card callback
                // even if the slot was empty before creative restoration.
                if let Some(old)=staged.upgrades[0].take() { changes.replaced_tracked.push(old); }
                mark_card_removed(&mut staged);upgrade_callback=true;
                staged.upgrades[0]=desired.as_ref().map(|s|moved_copy(s,1));
            }
            if let Some(mut removed)=staged.upgrades[0].take() {
                changes.replaced_tracked.push(removed.clone());
                horizon_client::ae2_stack_identity::detach_if_tracked(&mut removed);
                add_to_player(&mut inventory,&mut removed,64,&mut changes.replaced_tracked);
                if removed.count>0 {if let Some(tool)=tool.as_mut(){add_to_player(&mut tool.slots,&mut removed,64,&mut changes.replaced_tracked);}}
                if removed.count>0 {changes.drops.push(removed);}
                mark_card_removed(&mut staged);upgrade_callback=true;
            }
            if let Some(desired)=desired.as_ref() {
                let filter=ItemStack::new(&desired.item,1);
                let mut found=take_from_slots(&mut inventory,&filter,1,&mut changes,false);
                if found.is_empty() {if let Some(tool)=tool.as_mut(){found=take_from_slots(&mut tool.slots,&filter,1,&mut changes,true);}}
                if let Some(stack)=found.first() {
                    staged.upgrades[0]=Some(moved_copy(stack,1));upgrade_callback=true;
                    if stack.item.as_ref()!="ae2:crafting_card" {mark_card_removed(&mut staged);}
                } else { changes.missing=1; }
            }
        }
        let mut wake=upgrade_callback;
        if same {
            if let Some(fuzzy)=saved.fuzzy.as_ref(){staged.config.insert(FUZZY.into(),Tag::String(fuzzy.as_str().into()));wake=true;}
            if let Some(priority)=saved.priority {staged.config.insert("priority".into(),Tag::Int(priority));}
            if let Some((items,fluids))=saved.ghosts.as_ref(){staged.config.insert(ITEM_CONFIG.into(),items.clone());staged.config.insert(FLUID_CONFIG.into(),fluids.clone());wake=true;}
        }
        if wake {wake_interface(&mut staged);}
        *machine=staged;*player=inventory;*pocket=tool;Ok(changes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::super::super::ae2_interface::{set_interface_config,set_interface_fluid_config};

        #[test]
        fn interface_memory_transfers_ghosts_and_real_upgrade_without_copying_stock() {
            let mut source=Machine::new(MachineKind::Interface);
            source.config.insert(FUZZY.into(),Tag::String("percent_75".into()));
            source.config.insert("priority".into(),Tag::Int(123));
            let mut item=ItemStack::new("minecraft:diamond",3);item.data_mut().custom_name=Some("request".into());
            set_interface_config(&mut source,2,Some(&item)).unwrap();
            let water=super::super::super::ae2_fluids::fluid_key("minecraft:water");
            set_interface_fluid_config(&mut source,5,Some(&water)).unwrap();
            source.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));
            let mut card=ItemStack::new("ae2:memory_card",1);save_interface(&mut card,&source,Some("source")).unwrap();
            let saved=read_interface(&card).unwrap().unwrap();
            let mut target=Machine::new(MachineKind::Interface);
            target.slots[0]=Some(ItemStack::new("minecraft:gold_ingot",4));
            target.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));
            target.config.insert("ae2_interface_craft_links".into(),Tag::List(vec![Tag::compound([("Slot",Tag::Byte(2)),("cpu",Tag::IntArray(vec![1,64,2]))])]));
            let mut full=vec![Some(ItemStack::new("minecraft:stone",64));36];let mut pocket=None;
            let changed=apply_interface(&mut target,&mut full,&mut pocket,&saved,false,true).unwrap();
            assert_eq!(changed.missing,1);assert_eq!(changed.drops.len(),1);assert!(target.upgrades[0].is_none());
            assert_eq!(target.slots[0].as_ref().unwrap().count,4);
            assert_eq!(interface_config(&target).unwrap()[2].as_ref(),Some(&item));
            assert_eq!(interface_fluid_config(&target).unwrap()[5],Some((water,1000)));
            assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(123)));
            assert_eq!(target.config.get(&FUZZY.into()),Some(&Tag::String("percent_75".into())));
            assert_eq!(target.config.get(&"ae2_interface_cancelled_links".into()),Some(&Tag::Int(4)));
            assert_eq!(saved.name.as_deref(),Some("source"));
        }
        #[test]
        fn partial_interface_import_preserves_ghosts_and_returns_unsupported_card() {
            let mut target=Machine::new(MachineKind::Interface);
            let item=ItemStack::new("minecraft:emerald",8);set_interface_config(&mut target,3,Some(&item)).unwrap();
            target.config.insert("priority".into(),Tag::Int(-7));target.upgrades[0]=Some(ItemStack::new("ae2:fuzzy_card",1));
            let saved=partial_interface(&target).unwrap();let before_items=interface_config(&target).unwrap();
            let mut player=vec![None;36];player[0]=Some(ItemStack::new("ae2:speed_card",1));let mut pocket=None;
            let changed=apply_interface(&mut target,&mut player,&mut pocket,&saved,false,false).unwrap();
            assert!(changed.drops.is_empty());assert_eq!(changed.missing,0);assert!(target.upgrades[0].is_none());
            assert_eq!(interface_config(&target).unwrap(),before_items);assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(-7)));
            assert!(player.iter().flatten().any(|s|s.item.as_ref()=="ae2:speed_card"));
            assert!(player.iter().flatten().any(|s|s.item.as_ref()=="ae2:fuzzy_card"));
        }
        #[test]
        fn interface_part_memory_imports_generic_components_and_preserves_physical_stock() {
            let mut source=Machine::new(MachineKind::Interface);
            source.config.insert(FUZZY.into(),Tag::String("percent_75".into()));
            source.config.insert("priority".into(),Tag::Int(77));
            let ghost=ItemStack::new("minecraft:diamond",3);set_interface_config(&mut source,2,Some(&ghost)).unwrap();
            source.upgrades[0]=Some(ItemStack::new("ae2:fuzzy_card",1));
            let mut card=ItemStack::new("ae2:memory_card",1);
            save_interface_part(&mut card,&source,Some("source part")).unwrap();
            let saved=read_interface(&card).unwrap().unwrap();
            assert!(saved.part_source);assert_eq!(saved.name.as_deref(),Some("source part"));
            assert_eq!(interface_priority_from_card(&card).unwrap(),Some(77));

            let mut target=Machine::new(MachineKind::Interface);
            target.slots[0]=Some(ItemStack::new("minecraft:gold_ingot",4));
            target.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));
            target.config.insert("ae2_interface_craft_links".into(),Tag::List(vec![Tag::compound([("Slot",Tag::Byte(2)),("cpu",Tag::IntArray(vec![1,64,2]))])]));
            let mut player=vec![None;36];player[0]=Some(ItemStack::new("ae2:fuzzy_card",1));let mut pocket=None;
            // Generic Interface components transfer even though the part's
            // source description does not match the target block description.
            let changes=apply_interface(&mut target,&mut player,&mut pocket,&saved,false,true).unwrap();
            assert!(changes.drops.is_empty());assert_eq!(changes.missing,0);
            assert_eq!(target.slots[0].as_ref().unwrap().count,4);
            assert_eq!(interface_config(&target).unwrap()[2].as_ref(),Some(&ghost));
            assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(77)));
            assert_eq!(target.config.get(&FUZZY.into()),Some(&Tag::String("percent_75".into())));
            assert_eq!(target.upgrades[0].as_ref().unwrap().item.as_ref(),"ae2:fuzzy_card");
            assert!(player.iter().flatten().any(|s|s.item.as_ref()=="ae2:crafting_card"));
            assert_eq!(target.config.get(&"ae2_interface_cancelled_links".into()),Some(&Tag::Int(4)));
        }
        #[test]
        fn interface_part_partial_components_keep_absent_fields_and_distinguish_empty_upgrades() {
            let make_card=|fields:Vec<(&str,Tag)>| {
                let mut body=Compound::new();body.insert("source".into(),Tag::String(PART_SOURCE.into()));
                for (key,value) in fields {body.insert(key.into(),value);}
                let mut card=ItemStack::new("ae2:memory_card",1);
                card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::Compound(body))])));card
            };
            let mut target=Machine::new(MachineKind::Interface);target.slots[0]=Some(ItemStack::new("minecraft:gold_ingot",4));
            target.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));target.config.insert("priority".into(),Tag::Int(-7));
            let emerald=ItemStack::new("minecraft:emerald",8);set_interface_config(&mut target,3,Some(&emerald)).unwrap();
            target.config.insert("ae2_interface_craft_links".into(),Tag::List(vec![Tag::compound([("Slot",Tag::Byte(3)),("cpu",Tag::IntArray(vec![1,64,2]))])]));
            let original=target.clone();let mut player=vec![None;36];let mut pocket=None;
            let priority=read_interface(&make_card(vec![("priority",Tag::Int(123))])).unwrap().unwrap();
            let mut expected=original.clone();expected.config.insert("priority".into(),Tag::Int(123));
            let changes=apply_interface(&mut target,&mut player,&mut pocket,&priority,false,true).unwrap();
            assert_eq!(target,expected);assert!(changes.drops.is_empty());assert!(player.iter().all(Option::is_none));

            let empty=read_interface(&make_card(vec![("upgrades",Tag::List(vec![]))])).unwrap().unwrap();
            target=original.clone();let changes=apply_interface(&mut target,&mut player,&mut pocket,&empty,false,true).unwrap();
            assert!(target.upgrades[0].is_none());assert!(changes.drops.is_empty());
            assert!(player.iter().flatten().any(|s|s.item.as_ref()=="ae2:crafting_card"));
            assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(-7)));
            assert_eq!(interface_config(&target).unwrap()[3].as_ref(),Some(&emerald));
            assert_eq!(target.config.get(&"ae2_interface_cancelled_links".into()),Some(&Tag::Int(8)));

            let mut source=Machine::new(MachineKind::Interface);let diamond=ItemStack::new("minecraft:diamond",3);
            set_interface_config(&mut source,2,Some(&diamond)).unwrap();
            let items=source.config.get(&ITEM_CONFIG.into()).unwrap().clone();
            let ghosts=read_interface(&make_card(vec![("config_items",items),("config_fluids",Tag::List(vec![]))])).unwrap().unwrap();
            target=original.clone();let mut player=vec![None;36];apply_interface(&mut target,&mut player,&mut pocket,&ghosts,false,true).unwrap();
            assert_eq!(target.upgrades[0].as_ref().unwrap().item.as_ref(),"ae2:crafting_card");
            assert_eq!(interface_config(&target).unwrap()[2].as_ref(),Some(&diamond));
            assert!(interface_config(&target).unwrap()[3].is_none());assert_eq!(target.slots[0].as_ref().unwrap().count,4);
            assert!(!target.config.contains_key(&"ae2_interface_cancelled_links".into()));
        }
        #[test]
        fn generic_priority_is_independent_and_preserves_owned_interface_state() {
            let mut card=ItemStack::new("ae2:memory_card",1);
            card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([
                (KEY,Tag::compound([("source",Tag::String("ae2:interface".into())),("priority",Tag::Int(42)),("config_items",Tag::Int(999))]))
            ])));
            // Source components are independently imported. An unrelated
            // malformed ghost payload cannot cause priority to consume cards.
            assert_eq!(interface_priority_from_card(&card).unwrap(),Some(42));
            let mut target=Machine::new(MachineKind::Interface);
            target.slots[0]=Some(ItemStack::new("minecraft:gold_ingot",4));
            target.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));
            let ghost=ItemStack::new("minecraft:emerald",3);set_interface_config(&mut target,2,Some(&ghost)).unwrap();
            target.config.insert("ae2_interface_craft_links".into(),Tag::List(vec![Tag::compound([("Slot",Tag::Byte(2)),("cpu",Tag::IntArray(vec![1,64,2]))])]));
            let mut expected=target.clone();expected.config.insert("priority".into(),Tag::Int(42));
            apply_interface_priority(&mut target,42).unwrap();
            assert_eq!(target,expected);
            assert!(!target.config.contains_key(&"ae2_interface_cancelled_links".into()));

            let mut provider_card=ItemStack::new("ae2:memory_card",1);
            provider_card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([
                (KEY,Tag::compound([("source",Tag::String("ae2:pattern_provider".into())),("priority",Tag::Int(-9)),("patterns",Tag::Int(999))]))
            ])));
            assert_eq!(super::super::super::ae2_memory_provider::priority_from_card(&provider_card).unwrap(),Some(-9));
            assert_eq!(interface_priority_from_card(&provider_card).unwrap(),None);

            // Java generic import skips a missing priority component instead
            // of materializing a value or rejecting other compatible fields.
            let mut partial=ItemStack::new("ae2:memory_card",1);
            partial.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([
                (KEY,Tag::compound([("source",Tag::String("ae2:cable_interface".into()))]))
            ])));
            assert_eq!(interface_priority_from_card(&partial).unwrap(),None);
        }
    }
}
pub(in crate::game) use interface_memory::{SavedInterface,save_interface,save_interface_part,read_interface,interface_priority_from_card,apply_interface_priority,partial_interface,interface_has_components,interface_to_inscriber,interface_to_io,interface_to_vibration,apply_interface};
