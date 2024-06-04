use std::collections::{VecDeque, HashMap, HashSet};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher, DefaultHasher};

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use itertools::Itertools;

use super::{
    interface::{
        BasicMarkup, PipelineCommand, PipelineValues, SymbolTreeTable, SymbolTreeTableCell, SymbolTreeTableList, SymbolTreeTableNode,
        SymbolCrossrefInfo, SymbolTreeTableColumn,
    },
    symbol_graph::{
        DerivedSymbolInfo, SymbolGraphNodeId,
    },
};

use crate::file_format::analysis::{
    StructuredFieldInfo, StructuredBitPositionInfo,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum SymbolFormatMode {
    FieldLayout,
    // - class-field-use-matrix: table for each class, look up all its methods and all its
    //   fields, then filter the method "calls" to the fields.
    // - caller-matrix: look up a class, get all its methods.  look up all of
    //   the callers of all of those methods.  group them by their class.
    //   - row depth 0 is subsystem
    //   - row depth 1 is class or file if no class
    //   - row depth 2 is method/function
    //   - columns are the methods on the class, probably alphabetical.
    //     - columns could maybe have an upsell to the arg-matrix?
    //   - cells are a count.
    // - arg-matrix:
    //   - like caller-matrix but only for a single matrix and the columns are
    //     the args.
}

/// Given a list of symbol crossref infos, produce a SymbolTreeTable for display
/// purposes.
#[derive(Debug, Args)]
pub struct FormatSymbols {
    #[clap(long, value_parser, value_enum, default_value = "field-layout")]
    pub mode: SymbolFormatMode,
}

#[derive(Debug)]
pub struct FormatSymbolsCommand {
    pub args: FormatSymbols,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PlatformId(u32);

impl PlatformId {
    fn all() -> Self {
        Self(0)
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct PlatformGroupId(u32);

type ClassId = SymbolGraphNodeId;
type FieldId = SymbolGraphNodeId;

// A struct to represent single field and hole before the field,
// for specific platform.
#[derive(Clone, Eq, Hash, PartialEq)]
struct Field {
    class_id: ClassId,
    field_id: FieldId,
    type_pretty: String,
    pretty: String,
    lineno: u64,
    hole_bytes: Option<u32>,
    hole_after_base: bool,
    end_padding_bytes: Option<u32>,
    offset_bytes: u32,
    bit_positions: Option<StructuredBitPositionInfo>,
    size_bytes: Option<u32>,
}

impl Field {
    fn new(class_id: ClassId, field_id: FieldId,
           sym_info: &DerivedSymbolInfo, info: &StructuredFieldInfo) -> Self {
        Self {
            class_id: class_id,
            field_id: field_id,
            type_pretty: info.type_pretty.to_string(),
            pretty: info.pretty.to_string(),
            lineno: sym_info.get_def_lno(),
            hole_bytes: None,
            hole_after_base: false,
            end_padding_bytes: None,
            offset_bytes: info.offset_bytes,
            bit_positions: info.bit_positions.clone(),
            size_bytes: info.size_bytes.clone(),
        }
    }
}

struct ClassSize {
    per_platform: HashMap<PlatformId, u32>,
}

impl ClassSize {
    fn new() -> Self {
        Self {
            per_platform: HashMap::new(),
        }
    }

    fn set(&mut self, platform_id: PlatformId, size: u32) {
        self.per_platform.insert(platform_id, size);
    }
}

struct ClassSizeMap {
    map: HashMap<ClassId, ClassSize>,
}

impl ClassSizeMap {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    fn set(&mut self, platform_id: PlatformId, class_id: ClassId, size: u32) {
        if let Some(class_size) = self.map.get_mut(&class_id) {
            class_size.set(platform_id, size);
            return;
        }

        let mut class_size = ClassSize::new();
        class_size.set(platform_id, size);
        self.map.insert(class_id, class_size);
    }

    fn per_platform(&self, platform_id: &PlatformId) -> HashMap<ClassId, u32> {
        let mut result = HashMap::new();

        for (class_id, class_size) in &self.map {
            let size = class_size.per_platform.get(platform_id).unwrap().clone();
            result.insert(class_id.clone(), size);
        }

        result
    }
}

// A container for fields, with pre-calculated hash of fields.
struct FieldsWithHash {
    fields: Vec<Field>,
    hash: u64,
}

impl FieldsWithHash {
    fn new_with_field(field: Field) -> Self {
        Self {
            fields: vec![field],
            hash: 0,
        }
    }

    fn sort(&mut self) {
        self.fields.sort_by(|a, b| {
            let byte_result = a.offset_bytes.cmp(&b.offset_bytes);
            if byte_result != Ordering::Equal {
                return byte_result;
            }

            match (&a.bit_positions, &b.bit_positions) {
                (Some(a_pos), Some(b_pos)) => {
                    a_pos.begin.cmp(&b_pos.begin)
                }
                _ => byte_result
            }
        });
    }

    fn calculate_holes(&mut self, class_size_map: HashMap<ClassId, u32>) {
        let mut last_end_offset = 0;
        let mut last_index = 0;

        let len = self.fields.len();

        for index in 0..len {
            if self.fields[index].offset_bytes > last_end_offset {
                if index != last_index {
                    if self.fields[last_index].class_id != self.fields[index].class_id {
                        let last_class_id = &self.fields[last_index].class_id;
                        if let Some(size) = class_size_map.get(last_class_id) {
                            if last_end_offset < *size {
                                self.fields[last_index].end_padding_bytes = Some(size - last_end_offset);
                            }
                            last_end_offset = *size;
                        }

                        self.fields[index].hole_after_base = true;
                    }
                }

                if self.fields[index].offset_bytes > last_end_offset {
                    self.fields[index].hole_bytes = Some(self.fields[index].offset_bytes - last_end_offset);
                }
            }

            last_index = index;

            if let Some(pos) = &self.fields[index].bit_positions {
                let end = self.fields[index].offset_bytes + (pos.begin + pos.width + 7) / 8;
                if end > last_end_offset {
                    last_end_offset = end;
                }
                continue;
            }

            if let Some(size) = &self.fields[index].size_bytes {
                last_end_offset = self.fields[index].offset_bytes + size;
            }
        }

        if !self.fields.is_empty() {
            let last_class_id = &self.fields[last_index].class_id;
            if let Some(size) = class_size_map.get(last_class_id) {
                if last_end_offset < *size {
                    self.fields[last_index].end_padding_bytes = Some(size - last_end_offset);
                }
            }
        }
    }

    fn calculate_hash(&mut self) {
        let mut hasher = DefaultHasher::new();
        self.fields.hash(&mut hasher);
        self.hash = hasher.finish();
    }
}

// A struct to represent single class, with
// fields per each platform group.
struct Class {
    id: ClassId,
    name: String,
    fields: HashMap<FieldId, HashMap<PlatformGroupId, Field>>,
    merged_fields: Vec<Vec<Option<Field>>>,
}

impl Class {
    fn new(id: ClassId, name: String) -> Self {
        Self {
            id: id,
            name: name,
            fields: HashMap::new(),
            merged_fields: vec![],
        }
    }

    fn add_field(&mut self, group_id: PlatformGroupId, field: Field) {
        let field_id = field.field_id.clone();

        if let Some(field_variants_map) = self.fields.get_mut(&field_id) {
            field_variants_map.insert(group_id, field);
            return;
        }

        let mut field_variants_map = HashMap::new();
        field_variants_map.insert(group_id, field);
        self.fields.insert(field_id, field_variants_map);
    }

    fn finish_populating(&mut self, groups: &Vec<(PlatformGroupId, Vec<PlatformId>)>) {
        // Sort the fields based on:
        //   * Line number
        //   * Average bit offset of the field
        //   * Integer encoding of the groups where the field exists

        let mut field_list = vec![];

        for field_variants_map in self.fields.values() {
            let mut group_bits: u64 = 0;
            let mut total_lineno: u64 = 0;
            let mut total_bit_offset: u64 = 0;
            let mut field_count: u64 = 0;

            let mut field_variants = vec![];
            for (group_id, _) in groups {
                match field_variants_map.get(group_id) {
                    Some(field) => {
                        total_lineno += field.lineno;
                        total_bit_offset += (field.offset_bytes as u64) * 8;
                        if let Some(pos) = &field.bit_positions {
                            total_bit_offset += pos.begin as u64;
                        }
                        group_bits |= 1 << group_id.0;

                        field_count += 1;

                        field_variants.push(Some(field.clone()));
                    },
                    None => {
                        field_variants.push(None);
                    },
                }
            }

            let average_lineno = total_lineno / field_count;
            let average_bit_offset = total_bit_offset / field_count;

            field_list.push((average_lineno, average_bit_offset, group_bits, field_variants))
        }

        field_list.sort_by(|a, b| {
            let result = a.0.cmp(&b.0);
            if result != Ordering::Equal {
                return result;
            }

            let result = a.1.cmp(&b.1);
            if result != Ordering::Equal {
                return result;
            }

            let result = a.2.cmp(&b.2);
            if result != Ordering::Equal {
                return result;
            }

            Ordering::Equal
        });

        self.merged_fields = field_list
            .into_iter()
            .map(|(_, _, _, field_variants)| field_variants)
            .collect();
    }
}

// Collect all platforms appeared in the analysis.
struct PlatformMap {
    platform_id_to_name: Vec<String>,

    // The temporary data structure to calculate platform ID.
    platform_name_to_id: HashMap<String, PlatformId>,
}

impl PlatformMap {
    fn new() -> Self {
        Self {
            platform_id_to_name: vec![],
            platform_name_to_id: HashMap::new(),
        }
    }

    fn add(&mut self, platform: String) -> PlatformId {
        if let Some(platform_id) = self.platform_name_to_id.get(&platform) {
            return platform_id.clone();
        }

        let platform_id = PlatformId(self.platform_name_to_id.len() as u32);
        self.platform_id_to_name.push(platform.clone());
        self.platform_name_to_id.insert(platform, platform_id.clone());

        platform_id
    }

    fn finish_populating(&mut self) {
        if self.is_empty() {
            let id = self.add("All platforms".to_string());
            assert!(id == PlatformId::all());
        }
    }

    fn is_empty(&self) -> bool {
        self.platform_name_to_id.is_empty()
    }

    fn get(&self, platform: String) -> PlatformId {
        self.platform_name_to_id.get(&platform).unwrap().clone()
    }

    fn platform_ids(&self) -> Vec<PlatformId> {
        self.platform_id_to_name
            .iter()
            .enumerate()
            .map(|(i, _)| PlatformId(i as u32))
            .collect()
    }


    fn get_name(&self, platform_id: &PlatformId) -> String {
        self.platform_id_to_name[platform_id.0 as usize].clone()
    }
}

fn platform_name_to_order(name: &String) -> u32 {
    if name.starts_with("win") {
        return 0;
    }
    if name.starts_with("macosx") {
        return 1;
    }
    if name.starts_with("linux") {
        return 2;
    }
    if name.starts_with("android") {
        return 3;
    }
    if name.starts_with("ios") {
        return 4;
    }
    return 5;
}

// Struct to hold the list of fields for the entire class hierarchy
// per platform, and calculate the hole between them.
struct FieldsPerPlatform {
    fields_per_platform: HashMap<PlatformId, FieldsWithHash>,
}

impl FieldsPerPlatform {
    fn new() -> Self {
        Self {
            fields_per_platform: HashMap::new(),
        }
    }

    fn add_field(&mut self, platform_id: &PlatformId, field: Field) {
        if let Some(fields) = self.fields_per_platform.get_mut(platform_id) {
            fields.fields.push(field);
            return;
        }

        self.fields_per_platform.insert(platform_id.clone(), FieldsWithHash::new_with_field(field));
    }

    // Once all fields are populated, process them for further operation.
    fn finish_populating(&mut self, class_size_map: &ClassSizeMap,
                         has_multiple_inheritance: bool) {
        for (platform_id, fields) in self.fields_per_platform.iter_mut() {
            fields.sort();
            if !has_multiple_inheritance {
                fields.calculate_holes(class_size_map.per_platform(&platform_id));
            }
            fields.calculate_hash();
        }
    }

    fn group_platforms(&self, platform_map: &PlatformMap) -> Vec<(PlatformGroupId, Vec<PlatformId>)> {
        if self.fields_per_platform.is_empty() {
            // If all fields are platform-agnostic, simply return them.
            return vec![(PlatformGroupId(0), platform_map.platform_ids())];
        }

        // Group platforms by fields.
        let mut groups: Vec<(u64, Vec<PlatformId>)> = vec![];

        let mut platform_ids = platform_map.platform_ids();

        // Make the order consistent as much as possible across classes.
        platform_ids.sort_by(|a, b| {
            let a_name = platform_map.get_name(&a);
            let b_name = platform_map.get_name(&b);

            let a_order = platform_name_to_order(&a_name);
            let b_order = platform_name_to_order(&b_name);

            let result = a_order.cmp(&b_order);
            if result != Ordering::Equal {
                return result
            }

            a_name.cmp(&b_name)
        });

        'next_platform: for platform_id in &platform_ids {
            if let Some(fields) = self.fields_per_platform.get(&platform_id) {
                for (hash, platforms) in &mut groups {
                    if fields.hash == *hash {
                        let existing = &self.fields_per_platform.get(&platforms[0]).unwrap().fields;
                        if fields.fields == *existing {
                            platforms.push(platform_id.clone());
                            continue 'next_platform;
                        }
                    }
                }

                groups.push((fields.hash, vec![platform_id.clone()]));
            }
        }

        groups
            .into_iter()
            .enumerate()
            .map(|(i, (_, platforms))| (PlatformGroupId(i as u32), platforms))
            .collect()
    }

    fn get_fields_for_platforms<'a>(&'a self, platform_ids: &Vec<PlatformId>) -> Option<&'a Vec<Field>> {
        let platform_id = &platform_ids[0];
        self.fields_per_platform.get(&platform_id).map(|fields| &fields.fields)
    }
}

struct ClassMap {
    // All processed classes.
    class_map: HashMap<ClassId, Class>,

    // The list of classes, in the traverse order.
    class_list: Vec<ClassId>,

    // All platforms appeared inside the analysis.
    platform_map: PlatformMap,

    // Platforms grouped by the field layout.
    groups: Vec<(PlatformGroupId, Vec<PlatformId>)>,

    has_multiple_inheritance: bool,

    root_class_id: Option<ClassId>,
    stt: SymbolTreeTable,
}

impl ClassMap {
    fn new() -> Self {
        Self {
            class_map: HashMap::new(),
            class_list: vec![],
            platform_map: PlatformMap::new(),
            groups: vec![],
            has_multiple_inheritance: false,
            root_class_id: None,
            stt: SymbolTreeTable::new(),
        }
    }

    async fn populate(&mut self, nom_sym_info: SymbolCrossrefInfo,
                      server: &Box<dyn AbstractServer + Send + Sync>) -> Result<()> {
        let root_sym_id = self.populate_platform_map(nom_sym_info, server).await?;

        self.root_class_id = Some(root_sym_id.clone());

        let mut class_size_map = ClassSizeMap::new();

        let mut fields_per_platform = FieldsPerPlatform::new();

        let mut pending_ids = VecDeque::new();
        pending_ids.push_back(self.root_class_id.clone().unwrap());

        while let Some(class_id) = pending_ids.pop_front() {
            let sym_info = self.stt.node_set.get(&class_id);
            let depth = sym_info.depth;
            let structured = sym_info.get_structured().unwrap();

            let cls = Class::new(
                class_id.clone(),
                structured.pretty.to_string(),
            );

            if structured.supers.len() > 1 {
                self.has_multiple_inheritance = true;
            }

            for super_info in &structured.supers {
                let (super_id, _) = self.stt
                    .node_set
                    .ensure_symbol(&super_info.sym, server, depth + 1)
                    .await?;

                pending_ids.push_back(super_id);
            }
            self.class_list.push(cls.id.clone());
            self.class_map.insert(cls.id.clone(), cls);

            for (maybe_platform, s) in structured.per_platform() {
                let mut maybe_platform_id: Option<PlatformId> = None;

                if let Some(platform) = maybe_platform {
                    let platform_id = self.platform_map.get(platform.clone());
                    maybe_platform_id = Some(platform_id);
                }

                if let Some(size) = &s.size_bytes {
                    if let Some(platform_id) = &maybe_platform_id {
                        class_size_map.set(platform_id.clone(), class_id.clone(), *size);
                    } else {
                        for platform_id in self.platform_map.platform_ids() {
                            class_size_map.set(platform_id.clone(), class_id.clone(), *size);
                        }
                    }
                }

                for field in s.fields.clone() {
                    let (field_id, field_info) = self.stt
                        .node_set
                        .ensure_symbol(&field.sym, server, depth + 1)
                        .await?;

                    let field = Field::new(class_id.clone(), field_id.clone(), field_info, &field);

                    if let Some(platform_id) = &maybe_platform_id {
                        fields_per_platform.add_field(&platform_id, field.clone());
                    } else {
                        for platform_id in self.platform_map.platform_ids() {
                            fields_per_platform.add_field(&platform_id, field.clone());
                        }
                    }
                }
            }
        }

        fields_per_platform.finish_populating(&class_size_map,
                                              self.has_multiple_inheritance);

        self.groups = fields_per_platform.group_platforms(&self.platform_map);

        for (group_id, platforms) in &self.groups {
            if let Some(fields) = fields_per_platform.get_fields_for_platforms(platforms) {
                for field in fields {
                    let cls = self.class_map.get_mut(&field.class_id).unwrap();
                    cls.add_field(group_id.clone(), field.clone());
                }
            }
        }

        for cls in self.class_map.values_mut() {
            cls.finish_populating(&self.groups);
        }

        Ok(())
    }

    async fn populate_platform_map(&mut self, nom_sym_info: SymbolCrossrefInfo,
                                   server: &Box<dyn AbstractServer + Send + Sync>) -> Result<SymbolGraphNodeId> {
        let (root_sym_id, _) = self.stt.node_set.add_symbol(DerivedSymbolInfo::new(
            nom_sym_info.symbol,
            nom_sym_info.crossref_info,
            0,
        ));

        let mut pending_ids = VecDeque::new();
        pending_ids.push_back(root_sym_id.clone());

        while let Some(class_id) = pending_ids.pop_front() {
            let sym_info = self.stt.node_set.get(&class_id);
            let depth = sym_info.depth;
            let Some(structured) = sym_info.get_structured() else {
                continue;
            };

            for super_info in &structured.supers {
                let (super_id, _) = self.stt
                    .node_set
                    .ensure_symbol(&super_info.sym, server, depth + 1)
                    .await?;
                pending_ids.push_back(super_id.clone());
            }

            for (maybe_platform, _) in structured.per_platform() {
                if let Some(platform) = maybe_platform {
                    self.platform_map.add(platform.clone());
                }
            }
        }

        self.platform_map.finish_populating();

        Ok(root_sym_id)
    }

    fn generate_tables(mut self, tables: &mut Vec<SymbolTreeTable>) {
        self.stt.columns.push(SymbolTreeTableColumn {
            label: vec![BasicMarkup::Heading("Name".to_string())],
            colspan: 1,
        });
        self.stt.columns.push(SymbolTreeTableColumn {
            label: vec![BasicMarkup::Heading("Type".to_string())],
            colspan: 1,
        });

        self.stt.sub_columns.push(SymbolTreeTableColumn {
            label: vec![],
            colspan: 1,
        });
        self.stt.sub_columns.push(SymbolTreeTableColumn {
            label: vec![],
            colspan: 1,
        });

        for (_, platforms) in &self.groups {
            let label = platforms
                .iter()
                .map(|platform_id| self.platform_map.get_name(&platform_id))
                .join(" ")
                .to_owned();

            self.stt.columns.push(SymbolTreeTableColumn {
                label: vec![BasicMarkup::Heading(label)],
                colspan: 2,
            });

            self.stt.sub_columns.push(SymbolTreeTableColumn {
                label: vec![BasicMarkup::Text("Offset".to_string())],
                colspan: 1,
            });
            self.stt.sub_columns.push(SymbolTreeTableColumn {
                label: vec![BasicMarkup::Text("Size".to_string())],
                colspan: 1,
            });
        }

        let column_offset: usize = 1;

        let mut root_node: Option<SymbolTreeTableNode> = None;

        for class_id in &self.class_list {
            let cls = self.class_map.get(&class_id).unwrap();

            let is_root = cls.id == self.root_class_id.as_ref().unwrap().clone();

            let mut class_node = SymbolTreeTableNode {
                sym_id: Some(cls.id.clone()),
                label: vec![
                    BasicMarkup::Heading(
                        format!("{}{}",
                                cls.name,
                                if !is_root { " (base class)" } else { "" },
                        ))
                ],
                col_vals: vec![],
                children: vec![],
                colspan: (1 + column_offset + self.groups.len() * 2) as u32,
            };

            if self.has_multiple_inheritance && is_root {
                let warn_node = SymbolTreeTableNode {
                    sym_id: None,
                    label: vec![
                        BasicMarkup::ItalicText(
                            "(The multiple inheritance is not yet supported. The field offsets in base classes can be wrong, and holes/paddings are not calculated)".to_string()
                        )
                    ],
                    col_vals: vec![],
                    children: vec![],
                    colspan: (1 + column_offset + self.groups.len() * 2) as u32,
                };
                class_node.children.push(warn_node);
            }

            let field_prefix = format!("{}::", cls.name);

            for field_variants in &cls.merged_fields {
                let mut has_hole = false;
                for maybe_field in field_variants {
                    if let Some(field) = &maybe_field {
                        if field.hole_bytes.is_some() {
                            has_hole = true;
                            break;
                        }
                    }
                }

                if has_hole {
                    let mut hole_node = SymbolTreeTableNode {
                        sym_id: None,
                        label: vec![],
                        col_vals: vec![],
                        children: vec![],
                        colspan: 1,
                    };

                    hole_node.col_vals.push(SymbolTreeTableCell::empty());

                    for maybe_field in field_variants {
                        match maybe_field {
                            Some(field) => {
                                let hole_bytes = field.hole_bytes.unwrap_or(0);
                                if hole_bytes == 0 {
                                    hole_node.col_vals.push(SymbolTreeTableCell::empty_colspan(2));
                                    continue;
                                }

                                hole_node.col_vals.push(SymbolTreeTableCell::italic_text_colspan(format!(
                                    "{} byte{} hole{}",
                                    hole_bytes,
                                    if hole_bytes > 1 { "s" } else { "" },
                                    if field.hole_after_base {
                                        " after base class"
                                    } else {
                                        ""
                                    }
                                ), 2));
                            },
                            None => {
                                if maybe_field.is_none() {
                                    hole_node.col_vals.push(SymbolTreeTableCell::empty_colspan(2));
                                }
                            }
                        }
                    }

                    class_node.children.push(hole_node);
                }

                let mut field_node = SymbolTreeTableNode {
                    sym_id: None,
                    label: vec![],
                    col_vals: vec![],
                    children: vec![],
                    colspan: 1,
                };

                field_node.col_vals.push(SymbolTreeTableCell::empty());

                let mut type_labels = vec![];
                let mut type_label_set = HashSet::new();

                for maybe_field in field_variants {
                    match maybe_field {
                        Some(field) => {
                            if field_node.sym_id.is_none() {
                                field_node.sym_id = Some(field.field_id.clone());

                                let mut pretty = field.pretty.clone();
                                pretty = pretty.replace(&field_prefix, "");
                                field_node.label = vec![BasicMarkup::Text(format!("{}", pretty))];
                            }

                            let type_label = match &field.type_pretty.is_empty() {
                                false => format!("{}", field.type_pretty),
                                true => "".to_string(),
                            };
                            if !type_label_set.contains(&type_label) {
                                type_label_set.insert(type_label.clone());
                                type_labels.push(type_label);
                            }

                            if let Some(pos) = &field.bit_positions {
                                field_node.col_vals.push(SymbolTreeTableCell::text(format!(
                                    "@ {:#x} + {} bit{}",
                                    field.offset_bytes,
                                    pos.begin,
                                    if pos.begin > 1 { "s" } else { "" }
                                )));
                                field_node.col_vals.push(SymbolTreeTableCell::text(format!(
                                    "{} bit{}",
                                    pos.width,
                                    if pos.width > 1 { "s" } else { "" }
                                )));
                            } else {
                                field_node.col_vals.push(SymbolTreeTableCell::text(format!(
                                    "@ {:#x}",
                                    field.offset_bytes,
                                )));
                                field_node.col_vals.push(SymbolTreeTableCell::text(format!(
                                    "{}",
                                    field.size_bytes.unwrap_or(0),
                                )));
                            }
                        }
                        None => {
                            field_node.col_vals.push(SymbolTreeTableCell::empty());
                            field_node.col_vals.push(SymbolTreeTableCell::empty());
                        }
                    }
                }

                let mut first = true;
                for label in type_labels {
                    if !first {
                        field_node.col_vals[0].contents.push(BasicMarkup::Text(" | ".to_string()));
                        field_node.col_vals[0].contents.push(BasicMarkup::Newline);
                    }
                    first = false;
                    field_node.col_vals[0].contents.push(BasicMarkup::Text(label));
                }

                class_node.children.push(field_node);

                let mut has_end_padding = false;
                for maybe_field in field_variants {
                    if let Some(field) = &maybe_field {
                        if field.end_padding_bytes.is_some() {
                            has_end_padding = true;
                            break;
                        }
                    }
                }

                if has_end_padding {
                    let mut end_padding_node = SymbolTreeTableNode {
                        sym_id: None,
                        label: vec![],
                        col_vals: vec![],
                        children: vec![],
                        colspan: 1,
                    };

                    end_padding_node.col_vals.push(SymbolTreeTableCell::empty());

                    for maybe_field in field_variants {
                        match maybe_field {
                            Some(field) => {
                                let end_padding_bytes = field.end_padding_bytes.unwrap_or(0);
                                if end_padding_bytes == 0 {
                                    end_padding_node.col_vals.push(SymbolTreeTableCell::empty_colspan(2));
                                    continue;
                                }

                                end_padding_node.col_vals.push(SymbolTreeTableCell::italic_text_colspan(format!(
                                    "{} byte{} padding",
                                    end_padding_bytes,
                                    if end_padding_bytes > 1 { "s" } else { "" }
                                ), 2));
                            },
                            None => {
                                if maybe_field.is_none() {
                                    end_padding_node.col_vals.push(SymbolTreeTableCell::empty_colspan(2));
                                }
                            }
                        }
                    }

                    class_node.children.push(end_padding_node);
                }
            }

            match &mut root_node {
                Some(node) => {
                    node.children.push(class_node);
                },
                None => {
                    root_node = Some(class_node);
                }
            }
        }

        if let Some(node) = root_node {
            self.stt.rows.push(node);
        }
        tables.push(self.stt);
    }
}

#[async_trait]
impl PipelineCommand for FormatSymbolsCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let cil = match input {
            PipelineValues::SymbolCrossrefInfoList(cil) => cil,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "format-symbols needs a CrossrefInfoList".to_string(),
                }));
            }
        };

        match self.args.mode {
            SymbolFormatMode::FieldLayout => {
                let mut tables = vec![];

                for nom_sym_info in cil.symbol_crossref_infos {
                    let mut map = ClassMap::new();
                    map.populate(nom_sym_info, server).await?;
                    map.generate_tables(&mut tables);
                }

                Ok(PipelineValues::SymbolTreeTableList(SymbolTreeTableList {
                    tables,
                }))
            }
        }
    }
}
