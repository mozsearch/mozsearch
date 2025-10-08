use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use itertools::Itertools;
use serde_json::{from_str, Value};

use super::{
    interface::{
        PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolTreeTable,
        SymbolTreeTableAlignmentAndSize, SymbolTreeTableField, SymbolTreeTableFieldOffsetAndSize,
        SymbolTreeTableFieldType, SymbolTreeTableItem, SymbolTreeTableList, SymbolTreeTableNode,
    },
    symbol_graph::{DerivedSymbolInfo, SymbolGraphNodeId},
};

use crate::file_format::analysis::{
    AnalysisStructured, StructuredBitPositionInfo, StructuredFieldInfo,
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

    #[clap(long, value_parser)]
    pub show_cols: Option<String>,

    #[clap(long, value_parser)]
    pub hide_cols: Option<String>,
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

// The identifier for the specific class in the layout.
// Single class can get multiple TraversalId if the class
// appears multiple times in the hierarchy.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct TraversalId(u32);

// A struct to represent single field and hole before the field,
// for specific platform.
#[derive(Clone, Eq, Hash, PartialEq)]
struct Field {
    class_id: ClassId,
    class_traversal_id: TraversalId,
    class_end_offset: Option<u32>,
    field_id: Option<FieldId>,
    field_type_syms: Option<String>,
    type_pretty: String,
    pretty: String,
    def_path: String,
    start_lineno: u64,
    end_lineno: u64,
    hole_bytes: Option<u32>,
    hole_after_base: bool,
    end_padding_bytes: Option<u32>,
    offset_bytes: u32,
    bit_positions: Option<StructuredBitPositionInfo>,
    size_bytes: Option<u32>,
}

impl Field {
    #[allow(clippy::too_many_arguments)]
    fn new(
        class_id: ClassId,
        class_traversal_id: TraversalId,
        class_offset: u32,
        class_size: Option<u32>,
        field_id: FieldId,
        field_type_syms: String,
        struct_def_path: &Option<String>,
        identifier_lineno: u64,
        info: &StructuredFieldInfo,
    ) -> Self {
        let (def_path, start_lineno, end_lineno) = Self::parse_path_and_line_range(
            info.line_range.to_string(),
            struct_def_path,
            identifier_lineno,
        );

        Self {
            class_id,
            class_traversal_id,
            class_end_offset: class_size.map(|size| class_offset + size),
            field_id: Some(field_id),
            field_type_syms: Some(field_type_syms),
            type_pretty: info.type_pretty.to_string(),
            pretty: info.pretty.to_string(),
            def_path,
            start_lineno,
            end_lineno,
            hole_bytes: None,
            hole_after_base: false,
            end_padding_bytes: None,
            offset_bytes: class_offset + info.offset_bytes.unwrap_or(0),
            bit_positions: info.bit_positions.clone(),
            size_bytes: info.size_bytes,
        }
    }

    fn parse_path_and_line_range(
        s: String,
        struct_def_path: &Option<String>,
        identifier_lineno: u64,
    ) -> (String, u64, u64) {
        match s.split_once("#") {
            Some((path, range)) => {
                let def_path = if path.is_empty() {
                    // If the field is defined in the same file as struct itself,
                    // the path part is omitted.
                    struct_def_path.clone().unwrap_or("".to_string())
                } else {
                    path.to_string()
                };

                match range.split_once("-") {
                    Some((start, end)) => (
                        def_path,
                        start.parse().unwrap_or(identifier_lineno),
                        end.parse().unwrap_or(identifier_lineno),
                    ),
                    None => (
                        def_path,
                        range.parse().unwrap_or(identifier_lineno),
                        range.parse().unwrap_or(identifier_lineno),
                    ),
                }
            }
            None => (
                struct_def_path.clone().unwrap_or("".to_string()),
                identifier_lineno,
                identifier_lineno,
            ),
        }
    }

    fn new_vtable(
        class_id: ClassId,
        class_traversal_id: TraversalId,
        class_offset: u32,
        class_size: u32,
        size_bytes: u32,
    ) -> Self {
        Self {
            class_id,
            class_traversal_id,
            class_end_offset: Some(class_offset + class_size),
            field_id: None,
            field_type_syms: None,
            type_pretty: "".to_string(),
            pretty: "(vtable)".to_string(),
            def_path: "".to_string(),
            start_lineno: 0,
            end_lineno: 0,
            hole_bytes: None,
            hole_after_base: false,
            end_padding_bytes: None,
            offset_bytes: class_offset,
            bit_positions: None,
            size_bytes: Some(size_bytes),
        }
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
                (Some(a_pos), Some(b_pos)) => a_pos.begin.cmp(&b_pos.begin),
                _ => byte_result,
            }
        });
    }

    fn calculate_holes(&mut self) {
        let mut last_end_offset = 0;
        let mut last_index = 0;

        let len = self.fields.len();

        for index in 0..len {
            if self.fields[index].offset_bytes > last_end_offset {
                if index != last_index
                    && self.fields[last_index].class_traversal_id
                        != self.fields[index].class_traversal_id
                {
                    if let Some(end_offset) = &self.fields[last_index].class_end_offset.clone() {
                        if last_end_offset < *end_offset {
                            self.fields[last_index].end_padding_bytes =
                                Some(end_offset - last_end_offset);
                        }
                        last_end_offset = *end_offset;
                    }

                    self.fields[index].hole_after_base = true;
                }

                if self.fields[index].offset_bytes > last_end_offset {
                    self.fields[index].hole_bytes =
                        Some(self.fields[index].offset_bytes - last_end_offset);
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
            if let Some(end_offset) = &self.fields[last_index].class_end_offset {
                if last_end_offset < *end_offset {
                    self.fields[last_index].end_padding_bytes = Some(end_offset - last_end_offset);
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

struct FieldListItem {
    def_paths: String,
    average_lineno: u64,
    average_bit_offset: u64,
    group_bits: u64,
    field_variants: Vec<Option<Field>>,
}

struct AlignmentAndSize {
    alignment: Option<u32>,
    size: u32,
}

impl AlignmentAndSize {
    fn new(alignment: Option<u32>, size: u32) -> Self {
        Self { alignment, size }
    }
}

// A struct to represent single class, with
// fields per each platform group.
struct Class {
    id: ClassId,
    name: String,
    alignment_and_size: HashMap<PlatformId, AlignmentAndSize>,
    fields: HashMap<Option<FieldId>, HashMap<PlatformGroupId, Field>>,
    merged_fields: Vec<Vec<Option<Field>>>,
}

impl Class {
    fn new(id: ClassId, name: String) -> Self {
        Self {
            id,
            name,
            alignment_and_size: HashMap::new(),
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
            let mut field_def_paths = vec![];

            let mut field_variants = vec![];
            for (group_id, _) in groups {
                match field_variants_map.get(group_id) {
                    Some(field) => {
                        total_lineno += field.start_lineno;
                        total_bit_offset += (field.offset_bytes as u64) * 8;
                        if let Some(pos) = &field.bit_positions {
                            total_bit_offset += pos.begin as u64;
                        }
                        group_bits |= 1 << group_id.0;

                        field_count += 1;

                        field_variants.push(Some(field.clone()));

                        if !field_def_paths.contains(&field.def_path) {
                            field_def_paths.push(field.def_path.clone());
                        }
                    }
                    None => {
                        field_variants.push(None);
                    }
                }
            }

            let average_lineno = total_lineno / field_count;
            let average_bit_offset = total_bit_offset / field_count;

            field_list.push(FieldListItem {
                def_paths: field_def_paths.join(","),
                average_lineno,
                average_bit_offset,
                group_bits,
                field_variants,
            })
        }

        field_list.sort_by(|a, b| {
            if a.def_paths == b.def_paths {
                // If the fields are defined in the same file,
                // Sort them by line number, to sort
                // fields which exists only in certain platform
                // in right order.
                //
                // If the field comes from different file, for example when
                // `#include` is used, just ignore the line number and
                // compare with offset.
                let result = a.average_lineno.cmp(&b.average_lineno);
                if result != Ordering::Equal {
                    return result;
                }
            }

            let result = a.average_bit_offset.cmp(&b.average_bit_offset);
            if result != Ordering::Equal {
                return result;
            }

            let result = a.group_bits.cmp(&b.group_bits);
            if result != Ordering::Equal {
                return result;
            }

            Ordering::Equal
        });

        self.merged_fields = field_list
            .into_iter()
            .map(|item| item.field_variants)
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
            return *platform_id;
        }

        let platform_id = PlatformId(self.platform_name_to_id.len() as u32);
        self.platform_id_to_name.push(platform.clone());
        self.platform_name_to_id.insert(platform, platform_id);

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
        *self.platform_name_to_id.get(&platform).unwrap()
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

fn platform_name_to_order(name: &str) -> u32 {
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
    5
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

        self.fields_per_platform
            .insert(*platform_id, FieldsWithHash::new_with_field(field));
    }

    // Once all fields are populated, process them for further operation.
    fn finish_populating(&mut self, has_unsupported_multiple_inheritance: bool) {
        for (_, fields) in self.fields_per_platform.iter_mut() {
            fields.sort();
            if !has_unsupported_multiple_inheritance {
                fields.calculate_holes();
            }
            fields.calculate_hash();
        }
    }

    fn group_platforms(
        &self,
        platform_map: &PlatformMap,
    ) -> Vec<(PlatformGroupId, Vec<PlatformId>)> {
        if self.fields_per_platform.is_empty() {
            // If all fields are platform-agnostic, simply return them.
            return vec![(PlatformGroupId(0), platform_map.platform_ids())];
        }

        // Group platforms by fields.
        let mut groups: Vec<(u64, Vec<PlatformId>)> = vec![];

        let mut platform_ids = platform_map.platform_ids();

        // Make the order consistent as much as possible across classes.
        platform_ids.sort_by(|a, b| {
            let a_name = platform_map.get_name(a);
            let b_name = platform_map.get_name(b);

            let a_order = platform_name_to_order(&a_name);
            let b_order = platform_name_to_order(&b_name);

            let result = a_order.cmp(&b_order);
            if result != Ordering::Equal {
                return result;
            }

            a_name.cmp(&b_name)
        });

        'next_platform: for platform_id in &platform_ids {
            if let Some(fields) = self.fields_per_platform.get(platform_id) {
                for (hash, platforms) in &mut groups {
                    if fields.hash == *hash {
                        let existing = &self.fields_per_platform.get(&platforms[0]).unwrap().fields;
                        if fields.fields == *existing {
                            platforms.push(*platform_id);
                            continue 'next_platform;
                        }
                    }
                }

                groups.push((fields.hash, vec![*platform_id]));
            }
        }

        groups
            .into_iter()
            .enumerate()
            .map(|(i, (_, platforms))| (PlatformGroupId(i as u32), platforms))
            .collect()
    }

    fn get_fields_for_platforms<'a>(&'a self, platform_ids: &[PlatformId]) -> Option<&'a [Field]> {
        let platform_id = &platform_ids[0];
        self.fields_per_platform
            .get(platform_id)
            .map(|fields| fields.fields.as_slice())
    }
}

// A structure to represent the next item for the class hierarchy traversal.
struct TraversalItem {
    // The next class's ID.
    class_id: ClassId,

    // The offset for the class per platform.
    // Platforms not included in this map are not used for this traversal.
    offset_map: HashMap<PlatformId, u32>,
}

impl TraversalItem {
    fn new(class_id: ClassId) -> Self {
        Self {
            class_id,
            offset_map: HashMap::new(),
        }
    }

    fn add_offset(&mut self, platform_id: PlatformId, offset: u32) {
        self.offset_map.insert(platform_id, offset);
    }

    fn get_offset(&self, platform_id: &PlatformId) -> u32 {
        match self.offset_map.get(platform_id) {
            Some(offset) => *offset,
            None => 0,
        }
    }

    fn is_enabled(&self, platform_id: &PlatformId) -> bool {
        self.offset_map.contains_key(platform_id)
    }

    fn platforms(&self) -> Vec<PlatformId> {
        let mut result = vec![];
        for platform_id in self.offset_map.keys() {
            result.push(*platform_id);
        }
        result
    }
}

struct SupersMap {
    super_ids: Vec<ClassId>,
    supers: HashMap<ClassId, HashMap<PlatformId, u32>>,
}

impl SupersMap {
    fn new() -> Self {
        Self {
            super_ids: vec![],
            supers: HashMap::new(),
        }
    }

    fn add(&mut self, class_id: ClassId, platform_id: PlatformId, offset: u32) {
        if let Some(item) = self.supers.get_mut(&class_id) {
            item.insert(platform_id, offset);
            return;
        }

        let mut item = HashMap::new();
        item.insert(platform_id, offset);
        self.super_ids.push(class_id.clone());
        self.supers.insert(class_id, item);
    }

    fn into_traversal_items(self) -> Vec<TraversalItem> {
        let mut result = vec![];

        for class_id in self.super_ids {
            let offset_map = self.supers.get(&class_id).unwrap();
            let mut item = TraversalItem::new(class_id);
            for (platform_id, offset) in offset_map {
                item.add_offset(*platform_id, *offset);
            }
            result.push(item);
        }

        result
    }
}

struct ClassMap {
    // All processed classes.
    class_map: HashMap<TraversalId, Class>,

    // The list of classes, in the traverse order.
    class_list: Vec<TraversalId>,

    // All platforms appeared inside the analysis.
    platform_map: PlatformMap,

    // Platforms grouped by the field layout.
    groups: Vec<(PlatformGroupId, Vec<PlatformId>)>,

    // Formatted lines of each file referred from fields.
    file_lines: HashMap<String, Vec<String>>,

    has_unsupported_multiple_inheritance: bool,

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
            file_lines: HashMap::new(),
            has_unsupported_multiple_inheritance: false,
            root_class_id: None,
            stt: SymbolTreeTable::new(),
        }
    }

    async fn populate(
        &mut self,
        nom_sym_info: SymbolCrossrefInfo,
        server: &(dyn AbstractServer + Send + Sync),
    ) -> Result<()> {
        let root_sym_id = self.populate_platform_map(nom_sym_info, server).await?;

        self.root_class_id = Some(root_sym_id.clone());

        let mut fields_per_platform = FieldsPerPlatform::new();

        let mut root_item = TraversalItem::new(root_sym_id);
        for platform_id in self.platform_map.platform_ids() {
            root_item.add_offset(platform_id, 0);
        }

        let mut pending_items = VecDeque::new();
        pending_items.push_back(root_item);

        let mut traversal_index = 0;

        let mut has_multiple_inheritance = false;
        let mut has_non_zero_super_offset = false;

        while let Some(item) = pending_items.pop_front() {
            let class_id = item.class_id.clone();

            let sym_info = self.stt.node_set.get(&class_id);
            let depth = sym_info.depth;
            let Some(structured) = Self::get_struct_structured(sym_info) else {
                continue;
            };
            let struct_def_path = sym_info.get_def_path().cloned();

            let mut cls = Class::new(class_id.clone(), structured.pretty.to_string());

            let traversal_id = TraversalId(traversal_index);

            traversal_index += 1;

            self.class_list.push(traversal_id);

            let mut supers = SupersMap::new();

            for (maybe_platform, s) in structured.per_platform() {
                let mut maybe_platform_id: Option<PlatformId> = None;

                if let Some(platform) = maybe_platform {
                    let platform_id = self.platform_map.get(platform.clone());
                    if !item.is_enabled(&platform_id) {
                        continue;
                    }
                    maybe_platform_id = Some(platform_id);
                }

                let class_alignment = s.alignment_bytes;

                if let Some(class_size) = s.size_bytes {
                    if let Some(platform_id) = &maybe_platform_id {
                        cls.alignment_and_size.insert(
                            platform_id.clone(),
                            AlignmentAndSize::new(class_alignment, class_size),
                        );

                        if let Some(vtable_size_bytes) = &s.own_vf_ptr_bytes {
                            let offset = item.get_offset(platform_id);
                            let field = Field::new_vtable(
                                class_id.clone(),
                                traversal_id,
                                offset,
                                class_size,
                                *vtable_size_bytes,
                            );
                            fields_per_platform.add_field(platform_id, field.clone());
                        }
                    } else {
                        for platform_id in item.platforms() {
                            cls.alignment_and_size.insert(
                                platform_id,
                                AlignmentAndSize::new(class_alignment, class_size),
                            );

                            if let Some(vtable_size_bytes) = &s.own_vf_ptr_bytes {
                                let offset = item.get_offset(&platform_id);
                                let field = Field::new_vtable(
                                    class_id.clone(),
                                    traversal_id,
                                    offset,
                                    class_size,
                                    *vtable_size_bytes,
                                );
                                fields_per_platform.add_field(&platform_id, field.clone());
                            }
                        }
                    }
                }

                if s.supers.len() > 1 {
                    has_multiple_inheritance = true;
                }

                for super_info in &s.supers {
                    let (super_id, _) = self
                        .stt
                        .node_set
                        .ensure_symbol(&super_info.sym, server, depth + 1)
                        .await?;

                    if super_info.offset_bytes.unwrap_or(0) > 0 {
                        has_non_zero_super_offset = true;
                    }

                    if let Some(platform_id) = &maybe_platform_id {
                        let offset = item.get_offset(platform_id);
                        supers.add(
                            super_id.clone(),
                            *platform_id,
                            offset + super_info.offset_bytes.unwrap_or(0),
                        );
                    } else {
                        for platform_id in item.platforms() {
                            let offset = item.get_offset(&platform_id);
                            supers.add(
                                super_id.clone(),
                                platform_id,
                                offset + super_info.offset_bytes.unwrap_or(0),
                            );
                        }
                    }
                }

                for field in s.fields.clone() {
                    let (field_id, field_lineno) = {
                        let (field_id, field_info) = self
                            .stt
                            .node_set
                            .ensure_symbol(&field.sym, server, depth + 1)
                            .await?;

                        (field_id, field_info.get_def_lno())
                    };

                    let mut field_type_syms_vec = vec![];
                    let mut field_type_syms_set = HashSet::new();

                    // Add field type to the jumprefs, but we don't use the
                    // returned info.
                    if !field.type_sym.is_empty() {
                        let _ = self
                            .stt
                            .node_set
                            .ensure_symbol(&field.type_sym, server, depth + 1)
                            .await?;

                        field_type_syms_vec.push(field.type_sym.to_string());
                        field_type_syms_set.insert(field.type_sym);
                    }
                    for info in &field.pointer_info {
                        if field_type_syms_set.contains(&info.sym) {
                            continue;
                        }

                        let _ = self
                            .stt
                            .node_set
                            .ensure_symbol(&info.sym, server, depth + 1)
                            .await?;

                        field_type_syms_vec.push(info.sym.to_string());
                        field_type_syms_set.insert(info.sym);
                    }

                    let field_type_syms = field_type_syms_vec.iter().join(",");

                    if let Some(platform_id) = &maybe_platform_id {
                        let offset = item.get_offset(platform_id);
                        let field = Field::new(
                            class_id.clone(),
                            traversal_id,
                            offset,
                            s.size_bytes,
                            field_id.clone(),
                            field_type_syms,
                            &struct_def_path,
                            field_lineno,
                            &field,
                        );
                        self.populate_file_lines(&field.def_path, server).await?;
                        fields_per_platform.add_field(platform_id, field.clone());
                    } else {
                        for platform_id in item.platforms() {
                            let offset = item.get_offset(&platform_id);
                            let field = Field::new(
                                class_id.clone(),
                                traversal_id,
                                offset,
                                s.size_bytes,
                                field_id.clone(),
                                field_type_syms.clone(),
                                &struct_def_path,
                                field_lineno,
                                &field,
                            );
                            self.populate_file_lines(&field.def_path, server).await?;
                            fields_per_platform.add_field(&platform_id, field.clone());
                        }
                    }
                }
            }

            self.class_map.insert(traversal_id, cls);

            for super_item in supers.into_traversal_items() {
                pending_items.push_back(super_item);
            }
        }

        self.has_unsupported_multiple_inheritance =
            has_multiple_inheritance && !has_non_zero_super_offset;

        fields_per_platform.finish_populating(self.has_unsupported_multiple_inheritance);

        self.groups = fields_per_platform.group_platforms(&self.platform_map);

        for (group_id, platforms) in &self.groups {
            if let Some(fields) = fields_per_platform.get_fields_for_platforms(platforms) {
                for field in fields {
                    let cls = self.class_map.get_mut(&field.class_traversal_id).unwrap();
                    cls.add_field(*group_id, field.clone());
                }
            }
        }

        for cls in self.class_map.values_mut() {
            cls.finish_populating(&self.groups);
        }

        Ok(())
    }

    async fn populate_file_lines(
        &mut self,
        path: &String,
        server: &(dyn AbstractServer + Send + Sync),
    ) -> Result<()> {
        if path.is_empty() {
            return Ok(());
        }

        if self.file_lines.contains_key(path) {
            return Ok(());
        }

        let result = server.fetch_formatted_lines(path).await;
        if result.is_err() {
            return Ok(());
        }

        let (lines, sym_json) = result.unwrap();

        let syms: serde_json::Result<HashMap<String, Value>> = from_str(&sym_json);
        if let Ok(syms) = syms {
            for (sym, info) in syms {
                self.stt.extra_syms.insert(sym, info);
            }
        }

        self.file_lines.insert(path.clone(), lines);

        Ok(())
    }

    fn get_struct_structured(sym_info: &DerivedSymbolInfo) -> Option<AnalysisStructured> {
        let structured = sym_info.get_structured()?;

        // See clang TagTypeKind.
        // https://clang.llvm.org/doxygen/namespaceclang.html#a9237bdb3cf715b9bff8bcb3172635548
        if structured.kind != "struct"
            && structured.kind != "__interface"
            && structured.kind != "union"
            && structured.kind != "class"
            && structured.kind != "enum"
        {
            return None;
        }

        Some(structured)
    }

    async fn populate_platform_map(
        &mut self,
        nom_sym_info: SymbolCrossrefInfo,
        server: &(dyn AbstractServer + Send + Sync),
    ) -> Result<SymbolGraphNodeId> {
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
            let Some(structured) = Self::get_struct_structured(sym_info) else {
                continue;
            };

            for super_info in &structured.supers {
                let (super_id, _) = self
                    .stt
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
        for (_, platforms) in &self.groups {
            let label = platforms
                .iter()
                .map(|platform_id| self.platform_map.get_name(platform_id))
                .join(" ")
                .to_owned();

            self.stt.platforms.push(label);
        }

        for traversal_id in &self.class_list {
            let cls = self.class_map.get(traversal_id).unwrap();

            let is_root = cls.id == self.root_class_id.as_ref().unwrap().clone();

            let mut node_alignment_and_size = vec![];

            if is_root {
                for (_, platforms) in &self.groups {
                    let platform_id = platforms[0];

                    let (alignment, size) = match cls.alignment_and_size.get(&platform_id) {
                        Some(AlignmentAndSize { alignment, size }) => {
                            if let Some(alignment) = alignment {
                                (format!("align({})", alignment), format!("{}", size))
                            } else {
                                ("".to_string(), format!("{}", size))
                            }
                        }
                        None => ("".to_string(), "?".to_string()),
                    };

                    node_alignment_and_size
                        .push(SymbolTreeTableAlignmentAndSize::new(alignment, size));
                }
            }

            let mut class_node = SymbolTreeTableNode::new(
                cls.name.clone(),
                self.stt.node_set.get(&cls.id).symbol.to_string(),
                !is_root,
                node_alignment_and_size,
            );

            if self.has_unsupported_multiple_inheritance && is_root {
                class_node.items.push(
                    SymbolTreeTableItem::Warning(
                        "(This class has multiple inheritance but the offset is not found in the analysis file. The field offsets in base classes can be wrong, and holes/paddings are not calculated)".to_string()
                    )
                );
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
                    let mut holes = vec![];

                    for maybe_field in field_variants {
                        match maybe_field {
                            Some(field) => {
                                let hole_bytes = field.hole_bytes.unwrap_or(0);
                                if hole_bytes == 0 {
                                    holes.push(None);
                                    continue;
                                }

                                holes.push(Some(format!(
                                    "{} byte{} hole{}",
                                    hole_bytes,
                                    if hole_bytes > 1 { "s" } else { "" },
                                    if field.hole_after_base {
                                        " after base class"
                                    } else {
                                        ""
                                    }
                                )));
                            }
                            None => {
                                if maybe_field.is_none() {
                                    holes.push(None);
                                }
                            }
                        }
                    }

                    class_node.items.push(SymbolTreeTableItem::Hole(holes));
                }

                let mut field_name = "".to_string();
                let mut field_symbols = "".to_string();

                if let Some(field) = field_variants.iter().flatten().next() {
                    let pretty = field.pretty.clone();
                    field_name = pretty.replace(&field_prefix, "");

                    if let Some(field_id) = &field.field_id {
                        field_symbols = self.stt.node_set.get(field_id).symbol.to_string();
                    }
                }

                let mut field_item = SymbolTreeTableField::new(field_name, field_symbols);

                let mut type_label_set = HashSet::new();
                let mut path_and_range_set = HashSet::new();

                for maybe_field in field_variants {
                    match maybe_field {
                        Some(field) => {
                            if !type_label_set.contains(&field.type_pretty) {
                                type_label_set.insert(field.type_pretty.clone());

                                field_item.types.push(SymbolTreeTableFieldType::new(
                                    field.type_pretty.clone(),
                                    match &field.field_type_syms {
                                        Some(type_syms) => type_syms.clone(),
                                        None => "".to_string(),
                                    },
                                ));
                            }

                            let key =
                                (field.def_path.clone(), field.start_lineno, field.end_lineno);
                            if !path_and_range_set.contains(&key) {
                                path_and_range_set.insert(key);

                                if let Some(lines) = self.file_lines.get(&field.def_path) {
                                    for lineno in field.start_lineno..=field.end_lineno {
                                        if lineno == 0 {
                                            continue;
                                        }
                                        let index = lineno as usize - 1;
                                        if let Some(line) = lines.get(index) {
                                            field_item.lines.push(line.clone());
                                        }
                                    }
                                }
                            }

                            if let Some(pos) = &field.bit_positions {
                                field_item.offset_and_size.push(Some(
                                    SymbolTreeTableFieldOffsetAndSize::new(
                                        format!(
                                            "@ {:#x} + {} bit{}",
                                            field.offset_bytes,
                                            pos.begin,
                                            if pos.begin > 1 { "s" } else { "" }
                                        ),
                                        format!(
                                            "{} bit{}",
                                            pos.width,
                                            if pos.width > 1 { "s" } else { "" }
                                        ),
                                    ),
                                ))
                            } else {
                                field_item.offset_and_size.push(Some(
                                    SymbolTreeTableFieldOffsetAndSize::new(
                                        format!("@ {:#x}", field.offset_bytes,),
                                        format!("{}", field.size_bytes.unwrap_or(0),),
                                    ),
                                ));
                            }
                        }
                        None => {
                            field_item.offset_and_size.push(None);
                        }
                    }
                }

                class_node
                    .items
                    .push(SymbolTreeTableItem::Field(field_item));

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
                    let mut end_paddings = vec![];

                    for maybe_field in field_variants {
                        match maybe_field {
                            Some(field) => {
                                let end_padding_bytes = field.end_padding_bytes.unwrap_or(0);
                                if end_padding_bytes == 0 {
                                    end_paddings.push(None);
                                    continue;
                                }

                                end_paddings.push(Some(format!(
                                    "{} byte{} padding",
                                    end_padding_bytes,
                                    if end_padding_bytes > 1 { "s" } else { "" }
                                )));
                            }
                            None => {
                                if maybe_field.is_none() {
                                    end_paddings.push(None);
                                }
                            }
                        }
                    }

                    class_node
                        .items
                        .push(SymbolTreeTableItem::EndPadding(end_paddings));
                }
            }

            self.stt.rows.push(class_node);
        }

        tables.push(self.stt);
    }
}

#[async_trait]
impl PipelineCommand for FormatSymbolsCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
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

                let mut class_names = vec![];
                if let Some(cols) = &self.args.show_cols {
                    for col in cols.split(",") {
                        if col == "type" {
                            class_names.push(format!("show-{}", col));
                        }
                    }
                }
                if let Some(cols) = &self.args.hide_cols {
                    for col in cols.split(",") {
                        if col == "line" || col == "name" {
                            class_names.push(format!("hide-{}", col));
                        }
                    }
                }

                let class_name = if class_names.is_empty() {
                    None
                } else {
                    Some(class_names.join(" "))
                };

                Ok(PipelineValues::SymbolTreeTableList(SymbolTreeTableList {
                    tables,
                    class_name,
                }))
            }
        }
    }
}
