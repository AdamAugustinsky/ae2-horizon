// Staged for inclusion beside ae2_memory_emitter.rs after the Stage 68 build.
mod energy_emitter_memory {
    use super::*;
    use super::super::ae2_emitter::EnergyEmitterState;
    const SOURCE: &str = "ae2:energy_level_emitter";

    #[derive(Clone, Debug)]
    pub(in crate::game) struct SavedEnergyEmitter {
        pub matching: bool,
        pub threshold: Option<i64>,
        pub low: Option<bool>,
        pub has_upgrades: bool,
        pub name: Option<String>,
    }

    pub(in crate::game) fn save_energy_emitter(card: &mut ItemStack, state: &EnergyEmitterState, name: Option<&str>) -> Result<(), String> {
        if card.item.as_ref() != "ae2:memory_card" || card.count != 1 { return Err("invalid energy emitter card".into()); }
        let mut body=Compound::new();
        body.insert("source".into(), Tag::String(SOURCE.into()));
        body.insert("reporting_value".into(), Tag::Long(state.threshold));
        body.insert("settings".into(), Tag::compound([("redstone_emitter", Tag::String(if state.low {"low_signal"} else {"high_signal"}.into()))]));
        // EnergyLevelEmitterPart is upgradeable, but has zero slots. The
        // present empty component matters to generic targets with real cards.
        body.insert("upgrades".into(), Tag::List(Vec::new()));
        if let Some(name)=name {body.insert("name".into(),Tag::String(name.into()));}
        let mut custom=match card.data().and_then(|d|d.other("minecraft:custom_data")) {
            None=>Compound::new(), Some(Tag::Compound(v))=>v.clone(), _=>return Err("invalid card data".into()),
        };
        custom.remove(&P2P.into());custom.insert(KEY.into(),Tag::Compound(body));
        card.data_mut().set_other("minecraft:custom_data",Some(Tag::Compound(custom)));Ok(())
    }

    pub(in crate::game) fn read_energy_emitter(card: &ItemStack) -> Result<Option<SavedEnergyEmitter>, String> {
        if card.item.as_ref() != "ae2:memory_card" || card.count != 1 { return Err("invalid card".into()); }
        let Some(custom)=card.data().and_then(|d|d.other("minecraft:custom_data")) else{return Ok(None)};
        let Some(body)=custom.get(KEY) else{return Ok(None)};
        let Tag::Compound(_)=body else{return Err("invalid card settings".into())};
        let matching=body.get("source")==Some(&Tag::String(SOURCE.into()));
        let threshold=if matching {match body.get("reporting_value") {
            None=>None, Some(Tag::Long(v))=>Some(*v), _=>return Err("invalid emitter value".into()),
        }} else {None};
        let low=match body.get("settings") {
            None=>None,
            Some(Tag::Compound(settings))=>match settings.get(&"redstone_emitter".into()) {
                None=>None,
                Some(Tag::String(v))=>match v.to_string().map_err(|_|"invalid redstone setting")?.as_str() {
                    "low_signal"=>Some(true), "high_signal"=>Some(false), _=>None,
                },
                _=>return Err("invalid redstone setting type".into()),
            },
            _=>return Err("invalid emitter settings".into()),
        };
        let has_upgrades=match body.get("upgrades") {
            None=>false,
            Some(Tag::List(entries))=>{
                for tag in entries {let stack=save::item_from_nbt(tag).ok_or("invalid exported upgrade")?;if stack.count==0{return Err("invalid exported upgrade count".into())}}
                true
            },
            _=>return Err("invalid exported upgrades".into()),
        };
        let name=if matching {match body.get("name") {
            None=>None, Some(Tag::String(v))=>Some(v.to_string().map_err(|_|"invalid name")?), _=>return Err("invalid name".into()),
        }} else {None};
        // Ghosts, fuzzy/crafting settings and priority are not supported by
        // EnergyLevelEmitterPart and must not be decoded or applied here.
        Ok(Some(SavedEnergyEmitter{matching,threshold,low,has_upgrades,name}))
    }
    pub(in crate::game) fn energy_emitter_has_components(saved:&SavedEnergyEmitter)->bool {
        saved.low.is_some() || saved.has_upgrades
    }
    pub(in crate::game) fn apply_energy_emitter(state:&mut EnergyEmitterState, saved:&SavedEnergyEmitter) {
        if let Some(low)=saved.low {state.low=low;}
        if saved.matching {if let Some(value)=saved.threshold {state.threshold=value;}}
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::super::emitter_memory::{read_emitter,apply_emitter};
        use super::super::super::ae2_emitter::{EmitterFilter,EmitterState};
        #[test]
        fn energy_card_full_partial_and_unknown_components_follow_source() {
            let source=EnergyEmitterState{threshold:42,low:true};let mut card=ItemStack::new("ae2:memory_card",1);
            save_energy_emitter(&mut card,&source,Some("Source")).unwrap();let full=read_energy_emitter(&card).unwrap().unwrap();
            assert!(full.matching&&full.has_upgrades);assert_eq!(full.name.as_deref(),Some("Source"));
            let mut target=EnergyEmitterState{threshold:9,low:false};apply_energy_emitter(&mut target,&full);assert_eq!(target,source);
            let Some(Tag::Compound(mut custom))=card.data().unwrap().other("minecraft:custom_data").cloned() else{panic!("custom")};
            let Some(Tag::Compound(body))=custom.get_mut(&KEY.into()) else{panic!("body")};body.remove(&"reporting_value".into());body.remove(&"name".into());
            // Energy emitter does not implement a config inventory; unsupported
            // ghost data must not invalidate an otherwise valid settings import.
            body.insert("config_items".into(),Tag::String("ignored".into()));
            card.data_mut().set_other("minecraft:custom_data",Some(Tag::Compound(custom)));
            let partial=read_energy_emitter(&card).unwrap().unwrap();let mut target=EnergyEmitterState{threshold:9,low:false};apply_energy_emitter(&mut target,&partial);
            assert_eq!(target,EnergyEmitterState{threshold:9,low:true});assert_eq!(partial.name,None);
        }
        #[test]
        fn storage_card_changes_only_generic_energy_setting_and_never_consumes_cards() {
            let source=EmitterState{threshold:42,low:true,filter:Some(EmitterFilter::Item(ItemStack::new("minecraft:diamond",1))),fuzzy_mode:"ignore_all".into(),craft_via_redstone:false,upgrades:vec![Some(ItemStack::new("ae2:fuzzy_card",1))]};
            let mut card=ItemStack::new("ae2:memory_card",1);super::super::emitter_memory::save_emitter(&mut card,&source,Some("Source")).unwrap();
            let saved=read_energy_emitter(&card).unwrap().unwrap();assert!(!saved.matching);assert!(saved.has_upgrades);assert_eq!(saved.threshold,None);assert_eq!(saved.name,None);
            let mut target=EnergyEmitterState{threshold:9,low:false};apply_energy_emitter(&mut target,&saved);
            assert_eq!(target,EnergyEmitterState{threshold:9,low:true});
        }
        #[test]
        fn energy_card_empty_upgrades_return_real_storage_emitter_card() {
            let mut card=ItemStack::new("ae2:memory_card",1);save_energy_emitter(&mut card,&EnergyEmitterState{threshold:42,low:true},None).unwrap();
            let saved=read_emitter(&card).unwrap().unwrap();assert!(!saved.matching);
            let mut target=EmitterState{threshold:9,low:false,filter:Some(EmitterFilter::Item(ItemStack::new("minecraft:emerald",1))),fuzzy_mode:"ignore_all".into(),craft_via_redstone:false,upgrades:vec![Some(ItemStack::new("ae2:fuzzy_card",1))]};
            let mut player=vec![None;36];let mut pocket=None;let changes=apply_emitter(&mut target,&mut player,&mut pocket,&saved,false).unwrap();
            assert_eq!(target.threshold,9);assert!(target.low);assert!(target.upgrades[0].is_none());assert!(target.filter.is_some());assert!(changes.drops.is_empty());
            assert_eq!(player.iter().flatten().filter(|s|s.item.as_ref()=="ae2:fuzzy_card").map(|s|s.count).sum::<u32>(),1);
        }
    }
}
pub(in crate::game) use energy_emitter_memory::{SavedEnergyEmitter,save_energy_emitter,read_energy_emitter,energy_emitter_has_components,apply_energy_emitter};
