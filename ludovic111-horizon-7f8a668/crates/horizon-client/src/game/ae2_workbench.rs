// Source-aligned Cell Workbench template and physical-cell handoff.
const WORKBENCH_FILTERS:&str="ae2_workbench_filters";
const WORKBENCH_MODE:&str="ae2_workbench_copy_mode";

fn workbench_keep(machine:&horizon_client::ae2_world::Machine)->bool {
    text(&machine.config,WORKBENCH_MODE).as_deref()==Some("keep_on_remove")
}
fn workbench_decode(entries:&[Tag])->Result<Vec<Option<Tag>>,String>{
    let mut slots=horizon_client::ae2_cell_codec::decode_view_filter_slots(entries)?;
    for tag in slots.iter_mut().flatten(){if let Tag::Compound(c)=tag{c.remove(&"Slot".into());}}
    Ok(slots)
}
fn workbench_filters(machine:&horizon_client::ae2_world::Machine)->Result<Vec<Option<Tag>>,String>{
    let entries=match machine.config.get(&WORKBENCH_FILTERS.into()) {
        Some(Tag::List(entries))=>entries.clone(),
        Some(_)=>return Err("invalid workbench filters".into()),
        None=>match machine.slots.first().and_then(Option::as_ref) {
            None=>Vec::new(),
            Some(cell)=>match cell_settings(cell)?.get(&"filters".into()) {
                Some(Tag::List(entries))=>entries.clone(),
                _=>return Err("invalid cell filters".into()),
            }
        }
    };
    workbench_decode(&entries)
}
fn workbench_store_filters(machine:&mut horizon_client::ae2_world::Machine,slots:&[Option<Tag>])->Result<(),String>{
    let entries=horizon_client::ae2_cell_codec::encode_view_filter_slots(slots)?;
    machine.config.insert(WORKBENCH_FILTERS.into(),Tag::List(entries));Ok(())
}
fn workbench_filter_allowed(cell:&ItemStack,entry:&Tag)->bool{
    if entry.get("item").is_some() {
        return !is_fluid(cell) && entry.get("item").and_then(horizon_client::save::item_from_nbt)
            .is_some_and(|s|s.count>0);
    }
    if let (Some(Tag::String(id)),Some(Tag::ByteArray(_)))=(entry.get("fluid"),entry.get("components")){
        return (is_fluid(cell)||matches!(cell.item.as_ref(),"ae2:view_cell"|"ae2:creative_storage_cell"))
            && id.to_string().ok().is_some_and(|id|ae2_fluids::valid_fluid_id(&id));
    }
    false
}
fn workbench_apply_filters(cell:&mut ItemStack,slots:&[Option<Tag>])->Result<Vec<Option<Tag>>,String>{
    if slots.len()!=63{return Err("invalid workbench filter size".into())}
    let accepted:Vec<_>=slots.iter().map(|entry|entry.clone().filter(|v|workbench_filter_allowed(cell,v))).collect();
    let mut settings=cell_settings(cell)?;
    settings.insert("filters".into(),Tag::List(horizon_client::ae2_cell_codec::encode_view_filter_slots(&accepted)?));
    let mut staged=cell.clone();set_cell_settings(&mut staged,settings)?;
    if !is_valid_workbench_cell(&staged){return Err("invalid filtered workbench cell".into())}
    *cell=staged;Ok(accepted)
}
/// Invoke after a physical slot-zero change, using the previous workbench as
/// the fallback for legacy saves lacking a persistent template component.
pub(super) fn workbench_cell_changed(before:&horizon_client::ae2_world::Machine,after:&mut horizon_client::ae2_world::Machine)->Result<(),String>{
    if before.kind!=MachineKind::CellWorkbench||after.kind!=MachineKind::CellWorkbench{return Err("not a workbench".into())}
    let mut filters=workbench_filters(before)?;
    if let Some(cell)=after.slots.first_mut().and_then(Option::as_mut) {
        if !is_valid_workbench_cell(cell){return Err("invalid workbench cell".into())}
        let settings=cell_settings(cell)?;
        let Some(Tag::List(entries))=settings.get(&"filters".into()) else{return Err("invalid cell filters".into())};
        let installed=workbench_decode(entries)?;
        if installed.iter().any(Option::is_some){filters=installed}
        else{filters=workbench_apply_filters(cell,&filters)?;}
    }else if !workbench_keep(after){filters.fill(None);}
    workbench_store_filters(after,&filters)
}
fn workbench_partition(machine:&mut horizon_client::ae2_world::Machine)->Result<(),String>{
    let mut filters=vec![None;63];
    if let Some(cell)=machine.slots.first().and_then(Option::as_ref){
        if let Ok(storage)=read(cell) {
            for (slot,(key,_)) in filters.iter_mut().zip(storage.entries()){
                *slot=Some(if is_fluid(cell){
                    Tag::compound([("fluid",Tag::String(key.item_id.as_str().into())),("components",Tag::ByteArray(key.components.clone()))])
                }else{Tag::compound([("item",horizon_client::save::item_to_nbt(&ae2_lab::prototype(key)?))])});
            }
        }else if let Ok(keys)=creative_cell_keys(cell) {
            for (slot,key) in filters.iter_mut().zip(keys) {
                *slot=Some(match key {
                    CreativeCellKey::Item(key)=>Tag::compound([("item",horizon_client::save::item_to_nbt(&ae2_lab::prototype(&key)?))]),
                    CreativeCellKey::Fluid(key)=>Tag::compound([("fluid",Tag::String(key.item_id.as_str().into())),("components",Tag::ByteArray(key.components))]),
                });
            }
        }
    }
    if let Some(cell)=machine.slots.first_mut().and_then(Option::as_mut){filters=workbench_apply_filters(cell,&filters)?;}
    workbench_store_filters(machine,&filters)
}

#[cfg(test)]
mod workbench_template_tests {
    use super::*;
    use horizon_client::ae2_world::Machine;
    fn item_filter(id:&str)->Tag{Tag::compound([("item",horizon_client::save::item_to_nbt(&ItemStack::new(id,1)))])}
    fn one(id:&str,slot:usize)->Vec<Option<Tag>>{let mut out=vec![None;63];out[slot]=Some(item_filter(id));out}
    #[test]
    fn keep_template_copies_empty_cells_and_nonempty_incoming_cell_wins(){
        let mut before=Machine::new(MachineKind::CellWorkbench);
        before.config.insert(WORKBENCH_MODE.into(),Tag::String("keep_on_remove".into()));
        let template=one("minecraft:diamond",5);workbench_store_filters(&mut before,&template).unwrap();
        let mut inserted=before.clone();inserted.slots[0]=Some(ItemStack::new("ae2:item_storage_cell_1k",1));
        workbench_cell_changed(&before,&mut inserted).unwrap();assert_eq!(workbench_filters(&inserted).unwrap(),template);
        let mut removed=inserted.clone();removed.slots[0]=None;workbench_cell_changed(&inserted,&mut removed).unwrap();
        assert_eq!(workbench_filters(&removed).unwrap(),template);
        let mut next=removed.clone();next.slots[0]=Some(ItemStack::new("ae2:item_storage_cell_4k",1));workbench_cell_changed(&removed,&mut next).unwrap();
        assert_eq!(workbench_filters(&next).unwrap(),template);
        let emerald=one("minecraft:emerald",2);let mut other=ItemStack::new("ae2:item_storage_cell_1k",1);workbench_apply_filters(&mut other,&emerald).unwrap();
        let mut replaced=next.clone();replaced.slots[0]=Some(other);workbench_cell_changed(&next,&mut replaced).unwrap();
        assert_eq!(workbench_filters(&replaced).unwrap(),emerald);
    }
    #[test]
    fn clear_mode_removal_discards_only_template(){
        let mut before=Machine::new(MachineKind::CellWorkbench);before.slots[0]=Some(ItemStack::new("ae2:item_storage_cell_1k",1));
        workbench_store_filters(&mut before,&one("minecraft:diamond",3)).unwrap();
        let mut removed=before.clone();let physical=removed.slots[0].take().unwrap();workbench_cell_changed(&before,&mut removed).unwrap();
        assert!(workbench_filters(&removed).unwrap().iter().all(Option::is_none));assert_eq!(physical.item.as_ref(),"ae2:item_storage_cell_1k");
    }
    #[test]
    fn partition_reads_real_cell_keys_and_keeps_physical_stock(){
        let mut item=ItemStack::new("ae2:item_storage_cell_1k",1);let mut storage=read(&item).unwrap();
        for (id,count) in [("minecraft:diamond",4),("minecraft:gold_ingot",3)] {let key=ae2_lab::key(&ItemStack::new(id,1)).unwrap();assert_eq!(storage.insert(&key,count,Action::Modulate),count);}
        write(&mut item,&storage).unwrap();let mut machine=Machine::new(MachineKind::CellWorkbench);machine.slots[0]=Some(item);
        workbench_partition(&mut machine).unwrap();let filters=workbench_filters(&machine).unwrap();
        let keys:Vec<_>=filters.iter().flatten().filter_map(|v|v.get("item").and_then(horizon_client::save::item_from_nbt)).map(|s|s.item.to_string()).collect();
        assert_eq!(keys.len(),2);assert!(keys.contains(&"minecraft:diamond".into())&&keys.contains(&"minecraft:gold_ingot".into()));
        assert_eq!(read(machine.slots[0].as_ref().unwrap()).unwrap().total_items(),7);
    }
    #[test]
    fn template_copy_filters_incompatible_item_keys_from_fluid_cell(){
        let mut before=Machine::new(MachineKind::CellWorkbench);before.config.insert(WORKBENCH_MODE.into(),Tag::String("keep_on_remove".into()));
        let mut filters=one("minecraft:diamond",0);filters[4]=Some(Tag::compound([("fluid",Tag::String("minecraft:water".into())),("components",Tag::ByteArray(vec![]))]));workbench_store_filters(&mut before,&filters).unwrap();
        let mut inserted=before.clone();inserted.slots[0]=Some(ItemStack::new("ae2:fluid_storage_cell_1k",1));workbench_cell_changed(&before,&mut inserted).unwrap();
        let filtered=workbench_filters(&inserted).unwrap();assert!(filtered[0].is_none());assert_eq!(filtered[4].as_ref().unwrap().get("fluid"),Some(&Tag::String("minecraft:water".into())));
    }
}
