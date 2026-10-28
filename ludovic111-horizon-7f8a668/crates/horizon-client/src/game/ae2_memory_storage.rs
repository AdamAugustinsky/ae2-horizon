// Drive and ME Chest generic Memory Card components. Physical cells remain
// owned by the target. Chest terminal settings are persisted independently.
mod storage_memory {
    use super::*;
    const SORT:&str="sort_by";const VIEW:&str="view_mode";const DIRECTION:&str="sort_direction";
    const SORT_VALUES:&[&str]=&["name","amount","mod"];
    const VIEW_VALUES:&[&str]=&["all","stored","craftable"];
    const DIRECTION_VALUES:&[&str]=&["ascending","descending"];
    #[derive(Clone,Debug)] pub(in crate::game) struct SavedStorage {
        pub matching:bool,pub priority:Option<i32>,pub sort:Option<String>,pub view:Option<String>,pub direction:Option<String>,pub upgrades_present:bool,pub name:Option<String>,
    }
    fn is_storage(kind:MachineKind)->bool{matches!(kind,MachineKind::Drive|MachineKind::Chest)}
    fn setting(settings:Option<&Tag>,key:&str,allowed:&[&str])->Result<Option<String>,String>{
        let Some(settings)=settings else{return Ok(None)};let Tag::Compound(settings)=settings else{return Err("invalid storage settings".into())};
        match settings.get(&key.into()) {None=>Ok(None),Some(Tag::String(s))=>{let value=s.to_string().map_err(|_|"invalid storage setting text")?;Ok(allowed.contains(&value.as_str()).then_some(value))},_=>Err("invalid storage setting type".into())}
    }
    pub(in crate::game) fn save_storage(card:&mut ItemStack,machine:&Machine,name:Option<&str>)->Result<(),String>{
        if card.item.as_ref()!="ae2:memory_card"||card.count!=1||!is_storage(machine.kind){return Err("invalid storage card or host".into())}
        let priority=match machine.config.get(&"priority".into()){None=>0,Some(Tag::Int(v))=>*v,_=>return Err("invalid storage priority".into())};
        let mut body=Compound::new();body.insert("source".into(),Tag::String(machine.kind.id().into()));body.insert("priority".into(),Tag::Int(priority));
        if machine.kind==MachineKind::Chest {
            let sort=setting_value(machine,SORT,"name",SORT_VALUES)?;let view=setting_value(machine,VIEW,"all",VIEW_VALUES)?;let direction=setting_value(machine,DIRECTION,"ascending",DIRECTION_VALUES)?;
            body.insert("settings".into(),Tag::compound([(SORT,Tag::String(sort.as_str().into())),(VIEW,Tag::String(view.as_str().into())),(DIRECTION,Tag::String(direction.as_str().into()))]));
            // ITerminalHost exports an empty upgrade inventory on ME Chest.
            body.insert("upgrades".into(),Tag::List(Vec::new()));
        }
        if let Some(name)=name{body.insert("name".into(),Tag::String(name.into()));}
        let mut custom=match card.data().and_then(|d|d.other("minecraft:custom_data")){None=>Compound::new(),Some(Tag::Compound(v))=>v.clone(),_=>return Err("invalid card data".into())};
        custom.remove(&P2P.into());custom.insert(KEY.into(),Tag::Compound(body));card.data_mut().set_other("minecraft:custom_data",Some(Tag::Compound(custom)));Ok(())
    }
    pub(in crate::game) fn read_storage(card:&ItemStack,kind:MachineKind)->Result<Option<SavedStorage>,String>{
        if card.item.as_ref()!="ae2:memory_card"||card.count!=1||!is_storage(kind){return Err("invalid card or storage kind".into())}
        let Some(custom)=card.data().and_then(|d|d.other("minecraft:custom_data"))else{return Ok(None)};if !matches!(custom,Tag::Compound(_)){return Err("invalid card data".into())}
        let Some(body)=custom.get(KEY)else{return Ok(None)};if !matches!(body,Tag::Compound(_)){return Err("invalid storage card body".into())}
        let matching=body.get("source")==Some(&Tag::String(kind.id().into()));
        let priority=match body.get("priority"){None=>None,Some(Tag::Int(v))=>Some(*v),_=>return Err("invalid exported priority".into())};
        let (sort,view,direction,upgrades_present)=if kind==MachineKind::Chest {
            let settings=body.get("settings");let present=match body.get("upgrades"){None=>false,Some(Tag::List(_))=>true,_=>return Err("invalid exported upgrades".into())};
            (setting(settings,SORT,SORT_VALUES)?,setting(settings,VIEW,VIEW_VALUES)?,setting(settings,DIRECTION,DIRECTION_VALUES)?,present)
        }else{(None,None,None,false)};
        let name=if matching{match body.get("name"){None=>None,Some(Tag::String(v))=>Some(v.to_string().map_err(|_|"invalid name")?),_=>return Err("invalid name".into())}}else{None};
        Ok(Some(SavedStorage{matching,priority,sort,view,direction,upgrades_present,name}))
    }
    pub(in crate::game) fn storage_has_components(saved:&SavedStorage)->bool{saved.priority.is_some()||saved.sort.is_some()||saved.view.is_some()||saved.direction.is_some()||saved.upgrades_present}
    /// Source Drive priority and Chest terminal-setting listeners dirty their
    /// block; Chest priority and name assignment alone do not.
    pub(in crate::game) fn storage_import_marks_dirty(kind:MachineKind,saved:&SavedStorage)->bool{
        if kind==MachineKind::Drive{saved.priority.is_some()}else{kind==MachineKind::Chest&&(saved.sort.is_some()||saved.view.is_some()||saved.direction.is_some())}
    }
    pub(in crate::game) fn apply_storage(machine:&mut Machine,saved:&SavedStorage)->Result<(),String>{
        if !is_storage(machine.kind){return Err("invalid storage host".into())}
        if let Some(v)=saved.priority{machine.config.insert("priority".into(),Tag::Int(v));}
        if machine.kind==MachineKind::Chest {
            for (key,value) in [(SORT,&saved.sort),(VIEW,&saved.view),(DIRECTION,&saved.direction)] {if let Some(v)=value{machine.config.insert(key.into(),Tag::String(v.as_str().into()));}}
        }
        Ok(())
    }
    pub(in crate::game) fn storage_priority_from_card(card:&ItemStack)->Result<Option<i32>,String>{
        if card.item.as_ref()!="ae2:memory_card"||card.count!=1{return Err("invalid card".into())}
        let Some(data)=card.data().and_then(|d|d.other("minecraft:custom_data"))else{return Ok(None)};if !matches!(data,Tag::Compound(_)){return Err("invalid card data".into())}
        let Some(body)=data.get(KEY)else{return Ok(None)};
        if body.get("source")!=Some(&Tag::String("ae2:drive".into()))&&body.get("source")!=Some(&Tag::String("ae2:chest".into())){return Ok(None)}
        match body.get("priority"){None=>Ok(None),Some(Tag::Int(v))=>Ok(Some(*v)),_=>Err("invalid exported priority".into())}
    }
    /// Reuse the Interface generic decoder while ensuring a storage source
    /// never authorizes copying an Interface name or physical stock.
    pub(in crate::game) fn storage_as_interface(card:&ItemStack)->Result<Option<SavedInterface>,String>{
        let Some(mut data)=card.data().and_then(|d|d.other("minecraft:custom_data")).cloned()else{return Ok(None)};
        let Tag::Compound(ref mut custom)=data else{return Err("invalid card data".into())};let Some(Tag::Compound(body))=custom.get_mut(&KEY.into())else{return Ok(None)};
        if body.get(&"source".into())!=Some(&Tag::String("ae2:drive".into()))&&body.get(&"source".into())!=Some(&Tag::String("ae2:chest".into())){return Ok(None)}
        body.insert("source".into(),Tag::String("ae2:cable_interface".into()));let mut proxy=card.clone();proxy.data_mut().set_other("minecraft:custom_data",Some(data));read_interface(&proxy)
    }
    #[cfg(test)] mod tests {
        use super::*;
        #[test] fn storage_memory_cross_priority_keeps_target_cells_and_name(){
            let mut source=Machine::new(MachineKind::Drive);source.config.insert("priority".into(),Tag::Int(37));source.slots[0]=Some(ItemStack::new("ae2:item_storage_cell_1k",1));
            let mut card=ItemStack::new("ae2:memory_card",1);save_storage(&mut card,&source,Some("source")).unwrap();let saved=read_storage(&card,MachineKind::Chest).unwrap().unwrap();
            let mut target=Machine::new(MachineKind::Chest);target.slots[0]=Some(ItemStack::new("ae2:item_storage_cell_4k",1));target.energy=15.0;let physical=target.slots.clone();
            assert!(!saved.matching);assert_eq!(saved.name,None);apply_storage(&mut target,&saved).unwrap();assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(37)));assert_eq!(target.slots,physical);assert_eq!(target.energy,15.0);
        }
        #[test] fn chest_partial_components_and_empty_upgrades_match_source(){
            let mut source=Machine::new(MachineKind::Chest);source.config.insert("priority".into(),Tag::Int(37));source.config.insert(SORT.into(),Tag::String("amount".into()));source.config.insert(VIEW.into(),Tag::String("stored".into()));source.config.insert(DIRECTION.into(),Tag::String("descending".into()));
            let mut card=ItemStack::new("ae2:memory_card",1);save_storage(&mut card,&source,Some("chest")).unwrap();let saved=read_storage(&card,MachineKind::Chest).unwrap().unwrap();assert!(saved.matching&&saved.upgrades_present);assert_eq!(saved.name.as_deref(),Some("chest"));
            let mut target=Machine::new(MachineKind::Chest);target.config.insert("priority".into(),Tag::Int(-5));apply_storage(&mut target,&saved).unwrap();assert_eq!(target.config.get(&SORT.into()),Some(&Tag::String("amount".into())));
            let generic=storage_as_interface(&card).unwrap().unwrap();assert!(generic.part_source);assert_eq!(generic.priority,Some(37));assert!(matches!(generic.desired.as_ref(),Some(None)));
            let mut interface=Machine::new(MachineKind::Interface);interface.upgrades[0]=Some(ItemStack::new("ae2:crafting_card",1));let mut inv=vec![None;36];let mut pocket=None;let changes=apply_interface(&mut interface,&mut inv,&mut pocket,&generic,false,true).unwrap();
            assert!(changes.drops.is_empty());assert!(interface.upgrades[0].is_none());assert_eq!(inv.iter().flatten().filter(|s|s.item.as_ref()=="ae2:crafting_card").map(|s|s.count).sum::<u32>(),1);
        }
        #[test] fn chest_skips_invalid_setting_but_imports_other_fields(){
            let mut card=ItemStack::new("ae2:memory_card",1);card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::compound([
                ("source",Tag::String("ae2:chest".into())),("priority",Tag::Int(42)),("settings",Tag::compound([(SORT,Tag::String("invalid".into())),(VIEW,Tag::String("stored".into()))]))
            ]))])));
            let saved=read_storage(&card,MachineKind::Chest).unwrap().unwrap();assert!(saved.matching);assert_eq!(saved.sort,None);assert_eq!(saved.view.as_deref(),Some("stored"));
            let mut target=Machine::new(MachineKind::Chest);target.config.insert(SORT.into(),Tag::String("mod".into()));apply_storage(&mut target,&saved).unwrap();
            assert_eq!(target.config.get(&SORT.into()),Some(&Tag::String("mod".into())));assert_eq!(target.config.get(&VIEW.into()),Some(&Tag::String("stored".into())));assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(42)));
        }
        #[test] fn storage_dirty_follows_source_callback_cause(){
            let mut card=ItemStack::new("ae2:memory_card",1);card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::compound([("source",Tag::String("ae2:drive".into())),("priority",Tag::Int(7))]))])));
            let drive=read_storage(&card,MachineKind::Drive).unwrap().unwrap();let chest=read_storage(&card,MachineKind::Chest).unwrap().unwrap();assert!(storage_import_marks_dirty(MachineKind::Drive,&drive));assert!(!storage_import_marks_dirty(MachineKind::Chest,&chest));
            let mut body=Compound::new();body.insert("source".into(),Tag::String("ae2:chest".into()));body.insert("settings".into(),Tag::compound([(SORT,Tag::String("name".into()))]));
            card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::Compound(body))])));let equal=read_storage(&card,MachineKind::Chest).unwrap().unwrap();assert!(storage_import_marks_dirty(MachineKind::Chest,&equal));
            let mut empty=ItemStack::new("ae2:memory_card",1);empty.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::compound([("source",Tag::String("ae2:drive".into()))]))])));let name_only=read_storage(&empty,MachineKind::Drive).unwrap().unwrap();assert!(!storage_import_marks_dirty(MachineKind::Drive,&name_only));
        }
        #[test] fn storage_same_missing_priority_clears_name_but_not_priority(){
            let mut card=ItemStack::new("ae2:memory_card",1);card.data_mut().set_other("minecraft:custom_data",Some(Tag::compound([(KEY,Tag::compound([("source",Tag::String("ae2:drive".into()))]))])));
            let saved=read_storage(&card,MachineKind::Drive).unwrap().unwrap();assert!(saved.matching);assert_eq!(saved.priority,None);assert_eq!(saved.name,None);
            let mut target=Machine::new(MachineKind::Drive);target.config.insert("priority".into(),Tag::Int(-5));apply_storage(&mut target,&saved).unwrap();assert_eq!(target.config.get(&"priority".into()),Some(&Tag::Int(-5)));
        }
    }
}
pub(in crate::game) use storage_memory::{SavedStorage,save_storage,read_storage,storage_has_components,storage_import_marks_dirty,apply_storage,storage_priority_from_card,storage_as_interface};
