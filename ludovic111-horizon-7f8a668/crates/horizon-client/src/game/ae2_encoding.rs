//! Persisted face-owned Pattern Encoding Terminal configuration. Ghost values
//! are recipe descriptions only; physical blank/output stacks live in Machine.slots.
use super::ae2_processing::PatternFluid;
use horizon_client::{ae2_world::Machine,items::ItemStack,save};
use horizon_world::nbt::Tag;

#[derive(Clone,Debug,PartialEq)]
pub(super) struct EncodingState {
    pub inputs:Vec<Option<ItemStack>>,pub outputs:Vec<Option<ItemStack>>,
    pub fluid_inputs:Vec<Option<PatternFluid>>,pub fluid_outputs:Vec<Option<PatternFluid>>,
    pub substitutes:bool,pub fluid_substitutes:bool,pub processing:bool,pub special_mode:u8,pub stone_recipe:Option<String>,
}
impl Default for EncodingState {
    fn default()->Self {Self{inputs:vec![None;81],outputs:vec![None;27],fluid_inputs:vec![None;81],fluid_outputs:vec![None;27],substitutes:false,fluid_substitutes:true,processing:false,special_mode:0,stone_recipe:None}}
}
impl EncodingState {
    pub fn load(machine:&Machine)->Result<Self,String> {
        let Some(tag)=machine.config.get(&"ae2_encoding_state".into()) else{return Ok(Self::default())};
        if !matches!(tag,Tag::Compound(_)){return Err("invalid encoding state".into());}
        let mut result=Self::default();
        fn flag(tag:&Tag,key:&str,default:bool)->Result<bool,String> {match tag.get(key) {None=>Ok(default),Some(Tag::Byte(0))=>Ok(false),Some(Tag::Byte(1))=>Ok(true),_=>Err(format!("invalid {key}"))}}
        result.substitutes=flag(tag,"substitutes",false)?;result.fluid_substitutes=flag(tag,"fluid_substitutes",true)?;result.processing=flag(tag,"processing",false)?;
        result.special_mode=match tag.get("special_mode") {None=>0,Some(Tag::Byte(n)) if (0..=2).contains(n)=>*n as u8,_=>return Err("invalid encoding mode".into())};
        result.stone_recipe=match tag.get("stone_recipe") {None=>None,Some(Tag::String(id))=>Some(id.to_string().map_err(|_|"invalid stone recipe")?),_=>return Err("invalid stone recipe".into())};
        if result.processing && result.special_mode!=0 {return Err("conflicting encoding mode".into());}
        fn items(tag:&Tag,key:&str,dest:&mut [Option<ItemStack>])->Result<(),String> {
            let Some(value)=tag.get(key) else{return Ok(())};let Tag::List(entries)=value else{return Err(format!("invalid {key}"));};
            for entry in entries {let Some(Tag::Int(i))=entry.get("slot") else{return Err(format!("invalid {key} slot"));};let n=usize::try_from(*i).map_err(|_|format!("invalid {key} slot"))?;let target=dest.get_mut(n).ok_or_else(||format!("invalid {key} slot"))?;
                let stack=entry.get("item").and_then(save::item_from_nbt).filter(|s|s.count>0).ok_or_else(||format!("invalid {key} item"))?;
                if target.replace(stack).is_some(){return Err(format!("duplicate {key} slot"));}
            } Ok(())
        }
        fn fluids(tag:&Tag,key:&str,dest:&mut [Option<PatternFluid>])->Result<(),String> {
            let Some(value)=tag.get(key) else{return Ok(())};let Tag::List(entries)=value else{return Err(format!("invalid {key}"));};
            for entry in entries {let (Some(Tag::Int(i)),Some(Tag::String(name)),Some(Tag::ByteArray(components)),Some(Tag::Long(amount)))=(entry.get("slot"),entry.get("fluid"),entry.get("components"),entry.get("amount")) else{return Err(format!("invalid {key} fluid"));};
                let n=usize::try_from(*i).map_err(|_|format!("invalid {key} slot"))?;let target=dest.get_mut(n).ok_or_else(||format!("invalid {key} slot"))?;
                let name=name.to_string().map_err(|_|format!("invalid {key} fluid"))?;if name.is_empty() || *amount<=0{return Err(format!("invalid {key} fluid"));}
                if target.replace(PatternFluid{key:ae2_cell_core::ItemKey{item_id:name,components:components.clone()},amount:*amount as u64}).is_some(){return Err(format!("duplicate {key} slot"));}
            } Ok(())
        }
        items(tag,"inputs",&mut result.inputs)?;items(tag,"outputs",&mut result.outputs)?;
        fluids(tag,"fluid_inputs",&mut result.fluid_inputs)?;fluids(tag,"fluid_outputs",&mut result.fluid_outputs)?;
        if result.inputs.iter().zip(&result.fluid_inputs).any(|(a,b)|a.is_some()&&b.is_some()) || result.outputs.iter().zip(&result.fluid_outputs).any(|(a,b)|a.is_some()&&b.is_some()){return Err("overlapping item and fluid ghost slots".into());}
        Ok(result)
    }
    pub fn from_pattern(pattern:&ItemStack)->Option<Self> {
        let mut state=Self::default();
        if let Some(special)=super::ae2_processing::ae2_decode_special_pattern(pattern) {
            state.inputs[..9].clone_from_slice(&special.grid);state.substitutes=special.substitutes;
            state.special_mode=if special.kind=="smithing" {1}else{2};
            if state.special_mode==2 {state.stone_recipe=Some(special.recipe);}return Some(state);
        }
        if let Some((grid,_))=super::ae2_processing::decode_pattern(pattern) {
            state.inputs[..9].clone_from_slice(&grid);
            if let Some(data)=pattern.data().and_then(|d|d.other("minecraft:custom_data")).and_then(|v|v.get("ae2_lab_pattern")) {
                state.substitutes=matches!(data.get("substitutes"),Some(Tag::Byte(1)));
                state.fluid_substitutes=!matches!(data.get("fluid_substitutes"),Some(Tag::Byte(0)));
            }
            return Some(state);
        }
        let (inputs,outputs)=super::ae2_processing::ae2_decode_processing_pattern(pattern)?;
        state.processing=true;state.inputs=inputs;state.outputs=outputs;
        match super::ae2_processing::ae2_decode_processing_positions(pattern).ok()? {
            Some((fluids_in,fluids_out))=>{state.fluid_inputs=fluids_in;state.fluid_outputs=fluids_out;}
            None=>{
                // Existing patterns lacked slot metadata. Preserve their old
                // best-effort display; their mixed order cannot be recovered.
                let (fluids_in,fluids_out)=super::ae2_processing::ae2_decode_processing_fluids(pattern)?;
                for fluid in fluids_in {let i=state.inputs.iter().zip(&state.fluid_inputs).position(|(item,fluid)|item.is_none()&&fluid.is_none())?;state.fluid_inputs[i]=Some(fluid);}
                for fluid in fluids_out {let i=state.outputs.iter().zip(&state.fluid_outputs).position(|(item,fluid)|item.is_none()&&fluid.is_none())?;state.fluid_outputs[i]=Some(fluid);}
            }
        }
        Some(state)
    }
    pub fn save(&self,machine:&mut Machine)->Result<(),String> {
        if self.inputs.len()!=81 || self.outputs.len()!=27 || self.fluid_inputs.len()!=81 || self.fluid_outputs.len()!=27 {return Err("invalid encoding grid size".into());}
        let items=|values:&[Option<ItemStack>]|Tag::List(values.iter().enumerate().filter_map(|(i,s)|s.as_ref().map(|s|Tag::compound([("slot",Tag::Int(i as i32)),("item",save::item_to_nbt(s))]))).collect());
        let fluids=|values:&[Option<PatternFluid>]|Tag::List(values.iter().enumerate().filter_map(|(i,f)|f.as_ref().map(|f|Tag::compound([("slot",Tag::Int(i as i32)),("fluid",Tag::String(f.key.item_id.as_str().into())),("components",Tag::ByteArray(f.key.components.clone())),("amount",Tag::Long(f.amount as i64))]))).collect());
        if self.fluid_inputs.iter().chain(&self.fluid_outputs).flatten().any(|f|f.amount==0||f.amount>i64::MAX as u64||f.key.item_id.is_empty()){return Err("invalid encoding fluid".into());}
        machine.config.insert("ae2_encoding_state".into(),Tag::compound([("substitutes",Tag::Byte(i8::from(self.substitutes))),("fluid_substitutes",Tag::Byte(i8::from(self.fluid_substitutes))),("processing",Tag::Byte(i8::from(self.processing))),("special_mode",Tag::Byte(self.special_mode as i8)),("inputs",items(&self.inputs)),("outputs",items(&self.outputs)),("fluid_inputs",fluids(&self.fluid_inputs)),("fluid_outputs",fluids(&self.fluid_outputs))]));
        if let Some(recipe)=&self.stone_recipe {if recipe.is_empty(){return Err("invalid stone recipe".into());}if let Some(Tag::Compound(data))=machine.config.get_mut(&"ae2_encoding_state".into()){data.insert("stone_recipe".into(),Tag::String(recipe.as_str().into()));}}
        Self::load(machine)?;Ok(())
    }
}

/// A window stays bound to the exact physical part, not just its coordinates.
impl super::Game<'_> {
    pub(super) fn ae2_terminal_part_identity(&self,part:super::ae2_parts::PartAddress)->Option<(u64,[i32;4])> {
        let (dim,pos)=self.ae2_address_parts(part.host)?;let world=self.ae2_world_for(dim)?;
        let incarnation=world.block_entities.incarnation(&pos)?;
        let horizon_client::containers::BlockEntity::Ae2(host)=world.block_entities.get(&pos)? else{return None};
        let Tag::List(parts)=host.config.get(&"parts".into())? else{return None};
        let face=["east","west","up","down","south","north"].get(usize::from(part.face))?;
        let mut matches=parts.iter().filter(|v|v.get("face")==Some(&Tag::String((*face).into())));
        let value=matches.next()?;if matches.next().is_some(){return None;}
        let Some(Tag::String(id))=value.get("id") else{return None;};
        if !matches!(id.to_string().ok()?.as_str(),"ae2:terminal"|"ae2:crafting_terminal"|"ae2:pattern_encoding_terminal"){return None;}
        Some((incarnation,super::ae2_channels::part_identity(value)?))
    }
    pub(super) fn ae2_encoding_identity(&self,part:super::ae2_parts::PartAddress)->Option<(u64,[i32;4])> {
        let (dim,pos)=self.ae2_address_parts(part.host)?;let world=self.ae2_world_for(dim)?;
        let incarnation=world.block_entities.incarnation(&pos)?;
        let horizon_client::containers::BlockEntity::Ae2(host)=world.block_entities.get(&pos)? else{return None};
        let Tag::List(parts)=host.config.get(&"parts".into())? else{return None};
        let face=["east","west","up","down","south","north"].get(usize::from(part.face))?;
        let mut matches=parts.iter().filter(|v|v.get("face")==Some(&Tag::String((*face).into())));
        let value=matches.next()?;if matches.next().is_some(){return None;}
        if value.get("id")!=Some(&Tag::String("ae2:pattern_encoding_terminal".into())){return None;}
        Some((incarnation,super::ae2_channels::part_identity(value)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::ae2_world::MachineKind;
    #[test]
    fn positioned_mixed_pattern_restores_original_ghost_slots() {
        let mut state=EncodingState::default();state.processing=true;
        state.inputs[1]=Some(ItemStack::new("minecraft:iron_ingot",3));
        state.outputs[2]=Some(ItemStack::new("minecraft:gold_ingot",2));
        let fluid=PatternFluid{key:ae2_cell_core::ItemKey{item_id:"minecraft:water".into(),components:vec![0,42]},amount:750};
        state.fluid_inputs[8]=Some(fluid.clone());state.fluid_outputs[0]=Some(fluid);
        let encode=|state:&EncodingState|super::super::ae2_processing::ae2_encode_processing_positioned(
            &state.inputs,&state.outputs,&state.fluid_inputs,&state.fluid_outputs);
        let pattern=encode(&state).unwrap();assert_eq!(EncodingState::from_pattern(&pattern),Some(state.clone()));
        state.fluid_outputs[0]=None;assert!(encode(&state).is_err(),"a later item cannot replace an empty primary output");
    }
    #[test]
    fn special_mode_and_selected_recipe_survive_state_roundtrip() {
        let mut machine=Machine::new(MachineKind::PatternEncodingTerminal);let mut state=EncodingState::default();state.special_mode=2;state.stone_recipe=Some("minecraft:stone_slab".into());state.inputs[0]=Some(ItemStack::new("minecraft:stone",1));
        state.save(&mut machine).unwrap();assert_eq!(EncodingState::load(&machine).unwrap(),state);
        state.processing=true;assert!(state.save(&mut machine).is_err());
    }
    #[test]
    fn encoding_state_roundtrips_sparse_items_fluids_and_rejects_duplicate_slots() {
        let mut machine=Machine::new(MachineKind::PatternEncodingTerminal);
        let mut state=EncodingState::default();state.processing=true;state.substitutes=true;
        let mut item=ItemStack::new("minecraft:iron_ingot",3);item.data_mut().custom_name=Some("Tagged input".into());
        state.inputs[80]=Some(item);state.outputs[26]=Some(ItemStack::new("minecraft:gold_ingot",2));
        state.fluid_inputs[2]=Some(PatternFluid{key:ae2_cell_core::ItemKey{item_id:"example:water".into(),components:vec![0,42]},amount:750});
        state.save(&mut machine).unwrap();assert_eq!(EncodingState::load(&machine).unwrap(),state);
        let Tag::Compound(data)=machine.config.get_mut(&"ae2_encoding_state".into()).unwrap() else{panic!()};
        let Tag::List(inputs)=data.get_mut(&"inputs".into()).unwrap() else{panic!()};inputs.push(inputs[0].clone());
        assert!(EncodingState::load(&machine).is_err());
    }
}
