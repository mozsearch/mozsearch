/**
Prototyping pass on building a hierarchical graph representation akin to the
fancy-branch HierBuild/HierNode representation as defined and used in:
- https://github.com/asutherland/mozsearch/blob/fancy/ui/src/grokysis/frontend/diagramming/core_diagram.js
- https://github.com/asutherland/mozsearch/blob/fancy/ui/src/grokysis/frontend/diagramming/class_diagram.js
- https://github.com/asutherland/mozsearch/blob/fancy/ui/src/grok-ui/blockly/hiernode_generator.js

### Defining the Domain

#### Fancy Branch Prototyping Lessons Learned

Fancy branch prototyping resulted in 2 primary ideas for HierNode modeling:

1. Taking a node/edge graph and mapping the nodes into a hierarchical namespace
   and then applying various clustering approaches depending on heuristics that
   leveraged domain-knowledge (classes' ownership of methods/fields is well
   suited to table displays) and awareness of edge complexity (better to not
   use a table if there are a tons of intra-class edges!).

   - Although this didn't get prototyped, it was hypothesized that being able
     to view containment along file hierarchies could be useful either as its
     own axis or as something that could be interposed as an additional layer.
     It definitely would be useful if re-used for faceting, although that might
     not want to reuse the same graphing models.

2. The clustering presentation is also suitable for display of runtime behavior
   and data modeling, and this requires that we be able to have a concept of
   instancing of classes or conceptual objects (ex: windows for different
   origins).

#### Mapping

It seems like having a strongly typed path segment mechanism that can be
directly consumed at a presentation level without having to do any inference
would likely be useful.  Something like:
- PrettySymbol { pretty_segment, pretty_full, symbol }
- OSConcept { os_kind: [process, thread]}
- WebConcept { app_kind: [window, document, worker] }

Of course, having a strongly defined set of concepts in searchfox doesn't seem
useful, so we can perhaps unify that to:
- Concept { namespace, term, name }

This would give us:
- { namespace: "os", term: "thread", name: "main" }
- { namespace: "web", term: "window", name: "example.com" }
- { namespace: "web", term: "origin", name: "example.com" }

This could then be supplemented by an orthogonal instance tagging approach which
would tuple whatever it is attached to at every level of the hierarchy.  This
could be used for:
- Manually created examples.
- Runtime extraction from pernosco via pernosco-bridge where instances could
  could refer into pernosco-bridge's instance identifier map (which is something
  pernosco might be able to provide directly in the future).
- Runtime data retrieved from logs.  For example, GELAM/Workshop.
- Speculative: static specialization of values.  For example, discriminating
  between IPC enum states.

The ability of the concept to carry, for example, an origin name would probably
want to be symbiotic with the use of instances.  So a "window" with an origin
"example.com" would have an instance applied to it, as well as all of the
symbols emplaced within it.  The origin on the window would then be a
redundantly encoded piece of information for readability.  The window concept
would provide a label, whereas the instances would manifest as a color scheme
with an associated legend, but no direct text presentation.

### Relationship to Symbol Graphs; Internal Representation

The hierarchical representation is primarily a transformation from the raw
graph representation into something intended for human consumption.  We have no
foreseen need to be able to run efficient graph algorithms, so we don't need
to build our representation around petgraph.  In fact, our primary concern about
edges is being able to make sure they are emitted at the appropriate level of
the graphviz cluster hierarchy.
*/

pub struct PrettySymbolSegment {
    pretty_segment: String,
    pretty_full: String,
    symbols: Vec<String>,
}

pub struct ConceptSegment {
    namespace: String,
    term: String,
    name: String,
}

pub enum HierPathSegment {
    PrettySymbol(PrettySymbolSegment),
    Concept(ConceptSegment),
}

pub struct HierNode {
    id: String,
    segment: HierPathSegment,
    children: Vec<HierNode>,
}
