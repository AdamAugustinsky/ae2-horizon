//! Physical Memory Card transactions for supported machines and Interface faces.
//! Each part belongs to one cable face; unrelated host state is never replaced.
use super::Game;
use horizon_client::{containers::BlockEntity,ae2_network_tool::Inventory,ae2_world::{Machine,MachineKind},save};
use horizon_world::nbt::Tag;
enum Saved { Inscriber(super::ae2_memory::SavedInscriber), Io(super::ae2_memory::SavedIo), Vibration(super::ae2_memory::SavedVibration), Interface(super::ae2_memory::SavedInterface), Assembler(super::ae2_memory::SavedAssembler) }
enum Stored { Inscriber(super::ae2_memory::SavedInscriber), Io(super::ae2_memory::SavedIo), Vibration(super::ae2_memory::SavedVibration), Interface(super::ae2_memory::SavedInterface), Assembler(super::ae2_memory::SavedAssembler), Bus(super::ae2_memory::SavedBus) }
#[derive(Clone,Copy)] enum Target {Block, InterfacePart {index:usize,face:u8}}
struct Snapshot {target:Target,host:Machine,machine:Machine,incarnation:u64,name:Option<String>}
fn read_saved(card:&horizon_client::items::ItemStack)->Result<Option<Stored>,String> {
    if let Some(v)=super::ae2_memory::read_inscriber(card)? {return Ok(Some(Stored::Inscriber(v)))}
    if let Some(v)=super::ae2_memory::read_io(card)? {return Ok(Some(Stored::Io(v)))}
    if let Some(v)=super::ae2_memory::read_vibration(card)? {return Ok(Some(Stored::Vibration(v)))}
    if let Some(v)=super::ae2_memory::read_interface(card)? {return Ok(Some(Stored::Interface(v)))}
    if let Some(v)=super::ae2_memory::read_assembler(card)? {return Ok(Some(Stored::Assembler(v)))}
    if let Some(v)=super::ae2_memory::emitter_as_interface(card)? {return Ok(Some(Stored::Interface(v)))}
    if let Some(v)=super::ae2_memory::storage_as_interface(card)? {return Ok(Some(Stored::Interface(v)))}
    if let Some(id)=super::ae2_memory::bus_source_from_card(card) {
        if let Some(v)=super::ae2_memory::read_bus(card,id)? {return Ok(Some(Stored::Bus(v)))}
    }
    Ok(None)
}
#[derive(Clone,Copy)] enum ToolLocation {Inventory(usize),Armor(usize),Offhand}
impl Game<'_> {
    /// MemoryCardItem.use clears exported settings on alternate use in air.
    pub(super) fn ae2_clear_memory_in_air(&mut self) -> bool {
        if !self.keys.contains(&super::KeyCode::ShiftLeft) { return false; }
        let Some(mut card) = self.held().filter(|s| s.item.as_ref() == "ae2:memory_card" && s.count == 1).cloned() else { return false; };
        if super::ae2_memory::clear(&mut card).is_err() { return true; }
        horizon_client::ae2_stack_identity::synchronize_tracked(&card);
        *self.held_mut() = Some(card);
        self.system_message(horizon_client::text::Text::literal("Settings cleared"));
        true
    }
    fn memory_tool(&self)->Result<Option<(ToolLocation,Inventory)>,String>{
        for (i,item) in self.inventory.iter().enumerate(){if let Some(s)=item.as_ref().filter(|s|s.item.as_ref()=="ae2:network_tool") {return Ok(Some((ToolLocation::Inventory(i),Inventory::read(s)?)))}}
        for (i,item) in self.armor.iter().enumerate(){if let Some(s)=item.as_ref().filter(|s|s.item.as_ref()=="ae2:network_tool") {return Ok(Some((ToolLocation::Armor(i),Inventory::read(s)?)))}}
        if let Some(s)=self.offhand.as_ref().filter(|s|s.item.as_ref()=="ae2:network_tool") {return Ok(Some((ToolLocation::Offhand,Inventory::read(s)?)))}
        Ok(None)
    }
    fn machine_memory_snapshot(&self,pos:[i32;3],face:Option<&str>)->Result<Option<Snapshot>,String>{
        let key=(pos[0],pos[1],pos[2]);
        let Some(BlockEntity::Ae2(host))=self.world.block_entities.get(&key) else{return Ok(None)};
        let (target,machine,name)=if matches!(host.kind,MachineKind::Inscriber|MachineKind::IoPort|MachineKind::VibrationChamber|MachineKind::Interface|MachineKind::MolecularAssembler|MachineKind::Condenser|MachineKind::Drive|MachineKind::Chest) {
            if self.res.facts().get(self.world.block(pos[0],pos[1],pos[2])).name.as_ref()!=host.kind.id(){return Ok(None)}
            (Target::Block,(**host).clone(),self.world.block_names.get(&key).cloned())
        }else if host.kind==MachineKind::CableBus {
            let Some(face)=face else{return Ok(None)};
            let Some(face_index)=["east","west","up","down","south","north"].iter().position(|f|*f==face) else{return Err("invalid Interface face".into())};
            let Some(Tag::List(parts))=host.config.get(&"parts".into()) else{return Ok(None)};
            let mut found=parts.iter().enumerate().filter(|(_,part)|part.get("face")==Some(&Tag::String(face.into())));
            let Some((index,part))=found.next() else{return Ok(None)};
            if found.next().is_some(){return Err("duplicate Interface face".into())}
            if part.get("id")!=Some(&Tag::String("ae2:cable_interface".into())){return Ok(None)}
            super::ae2_channels::part_identity(part).ok_or("invalid Interface part identity")?;
            let stack=part.get("stack").and_then(save::item_from_nbt).filter(|s|s.item.as_ref()=="ae2:cable_interface"&&s.count==1).ok_or("invalid Interface part stack")?;
            let address=super::ae2_parts::PartAddress{host:self.ae2_local_address(key).ok_or("invalid host address")?,face:face_index as u8};
            let machine=self.ae2_part_machine(address)?.filter(|m|m.kind==MachineKind::Interface).ok_or("missing Interface part")?;
            (Target::InterfacePart{index,face:face_index as u8},machine,stack.custom_name().map(str::to_owned))
        }else{return Ok(None)};
        Ok(Some(Snapshot{target,host:(**host).clone(),machine,incarnation:self.world.block_entities.incarnation(&key).ok_or("missing machine identity")?,name}))
    }
    /// Commit one staged machine and optional name against the whole host
    /// snapshot and incarnation, keeping all other faces and host data intact.
    fn machine_memory_commit(&mut self,pos:[i32;3],before:&Snapshot,machine:Machine,name:Option<Option<String>>)->Result<(),String>{
        self.machine_memory_commit_with_dirty(pos,before,machine,name,true)
    }
    fn machine_memory_commit_with_dirty(&mut self,pos:[i32;3],before:&Snapshot,machine:Machine,name:Option<Option<String>>,mark_dirty:bool)->Result<(),String>{
        let key=(pos[0],pos[1],pos[2]);
        if self.world.block_entities.incarnation(&key)!=Some(before.incarnation){return Err("machine identity changed".into())}
        let mut next=match before.target {
            Target::Block=>machine,
            Target::InterfacePart{index,face}=>{
                let mut host=before.host.clone();
                let Some(Tag::List(parts))=host.config.get_mut(&"parts".into()) else{return Err("missing Interface part list".into())};
                let part=parts.get_mut(index).ok_or("missing Interface part")?;
                let expected=["east","west","up","down","south","north"][usize::from(face)];
                if part.get("face")!=Some(&Tag::String(expected.into())){return Err("Interface face changed".into())}
                let Tag::Compound(part)=part else{return Err("invalid Interface part".into())};
                part.insert("machine".into(),Tag::compound([
                    ("slots",save::encode_items(&machine.slots)),("upgrades",save::encode_items(&machine.upgrades)),("config",Tag::Compound(machine.config))]));
                if let Some(name)=name.as_ref(){
                    let mut stack=part.get(&"stack".into()).and_then(save::item_from_nbt).ok_or("invalid Interface stack")?;
                    stack.data_mut().custom_name=name.clone();part.insert("stack".into(),save::item_to_nbt(&stack));
                }
                host
            }
        };
        let Some(BlockEntity::Ae2(current))=self.world.block_entities.get_mut(&key) else{return Err("machine disappeared".into())};
        if **current!=before.host{return Err("machine changed during import".into())}
        std::mem::swap(&mut **current,&mut next);
        if matches!(before.target,Target::Block){if let Some(name)=name{match name{Some(name)=>{self.world.block_names.insert(key,name);},None=>{self.world.block_names.remove(&key);}}}}
        if mark_dirty {self.world.modified.insert((pos[0]>>4,pos[2]>>4));}Ok(())
    }
    pub(super) fn ae2_try_machine_memory_face(&mut self,pos:[i32;3],face:&str)->bool {
        if self.ae2_try_bus_memory_face(pos,face){return true}
        if self.ae2_try_energy_emitter_memory_face(pos,face){return true}
        if self.ae2_try_emitter_memory_face(pos,face){return true}
        self.ae2_try_machine_memory_at(pos,Some(face))
    }
    fn ae2_try_machine_memory_at(&mut self,pos:[i32;3],face:Option<&str>)->bool {
        let Some(mut card)=self.held().filter(|s|s.item.as_ref()=="ae2:memory_card"&&s.count==1).cloned() else{return false};
        let key=(pos[0],pos[1],pos[2]);
        let snapshot=match self.machine_memory_snapshot(pos,face){Ok(Some(v))=>v,Ok(None)=>return false,Err(_)=>return true};
        let before=&snapshot.machine;let kind=before.kind;
        if self.keys.contains(&super::KeyCode::ShiftLeft) {
            let name=snapshot.name.as_deref();
            let result=match kind {
                MachineKind::Inscriber=>super::ae2_memory::save_inscriber(&mut card,before,name),
                MachineKind::IoPort=>super::ae2_memory::save_io(&mut card,before,name),
                MachineKind::VibrationChamber=>super::ae2_memory::save_vibration(&mut card,before,name),
                MachineKind::Interface if matches!(snapshot.target,Target::InterfacePart{..})=>super::ae2_memory::save_interface_part(&mut card,before,name),
                MachineKind::Interface=>super::ae2_memory::save_interface(&mut card,before,name),
                MachineKind::MolecularAssembler=>super::ae2_memory::save_assembler(&mut card,before,name),
                MachineKind::Condenser=>super::ae2_memory::save_condenser(&mut card,before,name),
                MachineKind::Drive|MachineKind::Chest=>super::ae2_memory::save_storage(&mut card,before,name),_=>unreachable!()};
            if result.is_ok() {
                horizon_client::ae2_stack_identity::synchronize_tracked(&card);*self.held_mut()=Some(card);
                self.system_message(horizon_client::text::Text::literal("Settings saved"));
            }
            return true
        }
        use horizon_client::ae2_world::MachineKind as K;
        if matches!(kind,K::Drive|K::Chest) {
            let saved=match super::ae2_memory::read_storage(&card,kind){Ok(Some(v))=>v,_=>{self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}};
            if !saved.matching&&!super::ae2_memory::storage_has_components(&saved){self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}
            let mut machine=snapshot.machine.clone();if super::ae2_memory::apply_storage(&mut machine,&saved).is_err(){return true}
            // Drive priority setters dirty even for the same value. Chest
            // priority alone does not; its terminal-setting listeners do.
            let mark_dirty=super::ae2_memory::storage_import_marks_dirty(kind,&saved);
            let name=if saved.matching{Some(saved.name)}else{None};
            if self.machine_memory_commit_with_dirty(pos,&snapshot,machine,name,mark_dirty).is_err(){return true}
            // The source setter is invoked even for a repeated priority value;
            // current native routing reads the committed value on each operation.
            if let Some(priority)=saved.priority {if self.ae2_set_storage_priority_local(key,priority).is_err(){return true}}
            self.system_message(horizon_client::text::Text::literal(if saved.matching{"Settings loaded"}else{"Compatible settings restored"}));return true
        }
        if kind==K::Condenser {
            let saved=match super::ae2_memory::read_condenser(&card){Ok(Some(v))=>v,_=>{self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}};
            if !saved.matching && saved.output.is_none(){self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}
            let mut machine=snapshot.machine.clone();
            if let Some(mode)=saved.output.as_deref(){
                let max=match super::ae2_memory::condenser_output_item(mode){
                    Ok(Some(item))=>match self.res.items().get(item){Some(v)=>v.max_stack.max(1),None=>return true},
                    Ok(None)=>64,Err(_)=>return true,
                };
                if super::ae2_storage_devices::condenser_apply_memory_mode(&mut machine,mode,max).is_err(){return true}
            }
            let name=if saved.matching{Some(saved.name)}else{None};
            if self.machine_memory_commit(pos,&snapshot,machine,name).is_err(){return true}
            self.system_message(horizon_client::text::Text::literal(if saved.matching{"Settings loaded"}else{"Compatible settings restored"}));return true
        }
        if kind==K::Interface {
            match super::ae2_memory_provider::priority_from_card(&card) {
                Ok(Some(priority))=>{
                    let mut machine=snapshot.machine.clone();
                    if super::ae2_memory::apply_interface_priority(&mut machine,priority).is_err(){return true}
                    if self.machine_memory_commit(pos,&snapshot,machine,None).is_err(){return true}
                    self.system_message(horizon_client::text::Text::literal("Compatible priority restored"));
                    return true
                },
                Ok(None)=>{},
                Err(_)=>{self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true},
            }
        }
        let source=match read_saved(&card) {Ok(Some(v))=>v,_=>{self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}};
        if let Stored::Interface(saved)=&source {
            // On the generic path, an absent component performs no action.
            // Unlike machines only share upgrades; Interface shares the four
            // generic components and its part source never authorizes a name.
            let compatible=if kind==K::Interface {
                !saved.part_source || super::ae2_memory::interface_has_components(saved)
            }else{saved.desired.is_some()};
            if !compatible {self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true}
        }
        let (bus_io_redstone,bus_preserve_upgrades)=match &source {
            Stored::Bus(saved) if kind==K::IoPort=>(super::ae2_memory::bus_io_redstone(saved),saved.desired.is_none()),
            _=>(None,false)
        };
        if let Stored::Bus(saved)=&source {
            if kind!=K::Interface && saved.desired.is_none() && bus_io_redstone.is_none() {
                self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true
            }
        }
        // A part-saved Interface card is generically imported by either kind
        // of Interface target. It contains the same four generic components,
        // but its part description does not authorize copying the name.
        let interface_contents=kind==K::Interface && matches!(&source,Stored::Interface(_)|Stored::Bus(_));
        let (saved,same)=match (kind,source) {
            (K::Inscriber,Stored::Inscriber(v))=>(Some(Saved::Inscriber(v)),true),
            (K::Inscriber,Stored::Io(v))=>(super::ae2_memory::io_to_inscriber(before,&v).ok().map(Saved::Inscriber),false),
            (K::Inscriber,Stored::Vibration(v))=>(super::ae2_memory::vibration_to_inscriber(before,&v).ok().map(Saved::Inscriber),false),
            (K::Inscriber,Stored::Interface(_))=>(super::ae2_memory::interface_to_inscriber(before).ok().map(Saved::Inscriber),false),
            (K::Inscriber,Stored::Assembler(v))=>(super::ae2_memory::assembler_to_inscriber(before,&v).ok().map(Saved::Inscriber),false),
            (K::Inscriber,Stored::Bus(v))=>(super::ae2_memory::bus_to_inscriber(before,&v).ok().map(Saved::Inscriber),false),
            (K::IoPort,Stored::Io(v))=>(Some(Saved::Io(v)),true),
            (K::IoPort,Stored::Inscriber(v))=>(super::ae2_memory::inscriber_to_io(before,&v).ok().map(Saved::Io),false),
            (K::IoPort,Stored::Vibration(v))=>(super::ae2_memory::vibration_to_io(before,&v).ok().map(Saved::Io),false),
            (K::IoPort,Stored::Interface(_))=>(super::ae2_memory::interface_to_io(before).ok().map(Saved::Io),false),
            (K::IoPort,Stored::Assembler(v))=>(super::ae2_memory::assembler_to_io(before,&v).ok().map(Saved::Io),false),
            (K::IoPort,Stored::Bus(v))=>(super::ae2_memory::bus_to_io(before,&v).ok().map(Saved::Io),false),
            (K::VibrationChamber,Stored::Vibration(v))=>(Some(Saved::Vibration(v)),true),
            (K::VibrationChamber,Stored::Inscriber(v))=>(Some(Saved::Vibration(super::ae2_memory::inscriber_to_vibration(&v))),false),
            (K::VibrationChamber,Stored::Io(v))=>(Some(Saved::Vibration(super::ae2_memory::io_to_vibration(&v))),false),
            (K::VibrationChamber,Stored::Interface(_))=>(Some(Saved::Vibration(super::ae2_memory::interface_to_vibration())),false),
            (K::VibrationChamber,Stored::Assembler(v))=>(Some(Saved::Vibration(super::ae2_memory::assembler_to_vibration(&v))),false),
            (K::VibrationChamber,Stored::Bus(v))=>(Some(Saved::Vibration(super::ae2_memory::bus_as_vibration(&v))),false),
            (K::Interface,Stored::Interface(v))=>{let same=!v.part_source;(Some(Saved::Interface(v)),same)},
            (K::Interface,Stored::Bus(_))=>(super::ae2_memory::bus_as_interface(&card).ok().flatten().map(Saved::Interface),false),
            (K::Interface,_)=> (super::ae2_memory::partial_interface(before).ok().map(Saved::Interface),false),
            (K::MolecularAssembler,Stored::Assembler(v))=>(Some(Saved::Assembler(v)),true),
            (K::MolecularAssembler,Stored::Inscriber(v))=>(Some(Saved::Assembler(super::ae2_memory::inscriber_to_assembler(&v))),false),
            (K::MolecularAssembler,Stored::Io(v))=>(Some(Saved::Assembler(super::ae2_memory::io_to_assembler(&v))),false),
            (K::MolecularAssembler,Stored::Vibration(v))=>(Some(Saved::Assembler(super::ae2_memory::vibration_to_assembler(&v))),false),
            (K::MolecularAssembler,Stored::Interface(_))=>(Some(Saved::Assembler(super::ae2_memory::interface_to_assembler())),false),
            (K::MolecularAssembler,Stored::Bus(v))=>(Some(Saved::Assembler(super::ae2_memory::bus_to_assembler(&v))),false),
            _=>unreachable!(),
        };
        let Some(saved)=saved else{self.system_message(horizon_client::text::Text::literal("Invalid machine"));return true};
        let found=match self.memory_tool(){Ok(v)=>v,Err(_)=>return true};let location=found.as_ref().map(|(loc,_)|*loc);let mut pocket=found.map(|(_,inv)|inv);
        let mut machine=snapshot.machine.clone();let mut inventory=self.inventory.clone();let changes=match &saved {
            Saved::Inscriber(v) if same=>super::ae2_memory::apply_inscriber(&mut machine,&mut inventory,&mut pocket,v,self.creative),
            Saved::Inscriber(v)=>super::ae2_memory::apply_inscriber_partial(&mut machine,&mut inventory,&mut pocket,v,self.creative),
            Saved::Io(v) if same=>super::ae2_memory::apply_io(&mut machine,&mut inventory,&mut pocket,v,self.creative),
            Saved::Io(_) if bus_preserve_upgrades=>Ok(super::ae2_memory::PhysicalChanges{drops:Vec::new(),changed_tracked:Vec::new(),replaced_tracked:Vec::new(),missing:0}),
            Saved::Io(v)=>super::ae2_memory::apply_io_partial(&mut machine,&mut inventory,&mut pocket,v,self.creative),
            Saved::Vibration(v)=>super::ae2_memory::apply_vibration(&mut machine,&mut inventory,&mut pocket,v,self.creative),
            Saved::Interface(v)=>super::ae2_memory::apply_interface(&mut machine,&mut inventory,&mut pocket,v,self.creative,interface_contents),
            Saved::Assembler(v)=>super::ae2_memory::apply_assembler(&mut machine,&mut inventory,&mut pocket,v,self.creative)
        };let mut changes=match changes {Ok(v)=>v,Err(_)=>return true};
        if let Some(redstone)=bus_io_redstone{machine.config.insert("io_redstone".into(),Tag::String(redstone.into()));}
        // Validate and store the actual Network Tool before committing any of
        // the staged state; malformed pockets must not cause partial movement.
        let mut armor=self.armor.clone();let mut offhand=self.offhand.clone();
        if let (Some(location),Some(pocket))=(location,pocket.as_ref()) {
            let target=match location {ToolLocation::Inventory(i)=>inventory.get_mut(i).and_then(Option::as_mut),ToolLocation::Armor(i)=>armor.get_mut(i).and_then(Option::as_mut),ToolLocation::Offhand=>offhand.as_mut()};
            let Some(tool)=target else{return true};if pocket.store(tool).is_err(){return true}changes.changed_tracked.push(tool.clone());
        }
        let name=if same {Some(match &saved {Saved::Inscriber(v)=>v.name.clone(),Saved::Io(v)=>v.name.clone(),Saved::Vibration(v)=>v.name.clone(),Saved::Interface(v)=>v.name.clone(),Saved::Assembler(v)=>v.name.clone()})} else {None};
        if self.machine_memory_commit(pos,&snapshot,machine,name).is_err(){return true}
        self.inventory=inventory;self.armor=armor;self.offhand=offhand;
        if same && kind==MachineKind::IoPort {self.ae2_invalidate_variable_processor(key);}
        for stack in changes.replaced_tracked {horizon_client::ae2_stack_identity::mark_detached_by_replacement(&stack,None);}
        for stack in changes.changed_tracked {horizon_client::ae2_stack_identity::synchronize_tracked(&stack);}
        for stack in changes.drops {self.player_drop(stack);}
        if changes.missing>0 {self.system_message(horizon_client::text::Text::literal(format!("Missing {} upgrade cards",changes.missing)));}
        self.system_message(horizon_client::text::Text::literal(if same {"Settings loaded"} else {"Compatible settings restored"}));true
    }
}

include!("ae2_memory_emitter_host.rs");
include!("ae2_memory_energy_host.rs");
include!("ae2_memory_bus_host.rs");
