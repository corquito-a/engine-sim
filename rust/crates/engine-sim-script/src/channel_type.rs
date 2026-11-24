//! `piranha::ChannelType`: the little type lattice the scripting language uses.
//!
//! A channel type is a name plus an optional parent, and compatibility walks up
//! the parent chain. The original keeps these as raw pointers handed out by the
//! `scripting/` layer's registry, so identity matters; here a type is shared
//! through `Arc` and compares by name (which is what the hash comparison in
//! `ChannelType::isEqual` amounts to).

use std::sync::Arc;

#[derive(Debug)]
pub struct ChannelType {
    type_string: String,
    parent: Option<Arc<ChannelType>>,
    hash: i32,
}

impl ChannelType {
    /// `ChannelType::ChannelType()` -- the empty root type.
    pub fn new_root(type_string: impl Into<String>) -> Arc<ChannelType> {
        ChannelType::with_parent(type_string, None)
    }

    pub fn with_parent(
        type_string: impl Into<String>,
        parent: Option<Arc<ChannelType>>,
    ) -> Arc<ChannelType> {
        let type_string = type_string.into();
        let hash = generate_hash(&type_string);

        Arc::new(ChannelType { type_string, parent, hash })
    }

    pub fn type_string(&self) -> &str {
        &self.type_string
    }

    pub fn parent(&self) -> Option<&Arc<ChannelType>> {
        self.parent.as_ref()
    }

    /// `ChannelType::isEqual` -- the hash is compared first, then the strings.
    pub fn is_equal(&self, other: &ChannelType) -> bool {
        self.hash == other.hash && self.type_string == other.type_string
    }

    /// `ChannelType::isCompatibleWith` -- equal, or a parent is compatible.
    pub fn is_compatible_with(&self, other: &ChannelType) -> bool {
        if self.is_equal(other) {
            return true;
        }

        match &self.parent {
            Some(parent) => parent.is_compatible_with(other),
            None => false,
        }
    }

    /// Depth-first search of the chain for a type with this name, including
    /// itself. The registry uses it to intern types.
    pub fn find(&self, name: &str) -> Option<&ChannelType> {
        if self.type_string == name {
            return Some(self);
        }

        match &self.parent {
            Some(parent) => parent.find(name),
            None => None,
        }
    }

    /// The chain from this type up to the root, closest first.
    pub fn chain(&self) -> Vec<&ChannelType> {
        let mut chain = vec![self];
        let mut current = self;
        while let Some(parent) = &current.parent {
            chain.push(parent);
            current = parent;
        }
        chain
    }
}

impl PartialEq for ChannelType {
    fn eq(&self, other: &Self) -> bool {
        self.is_equal(other)
    }
}

impl Eq for ChannelType {}

impl std::hash::Hash for ChannelType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
        self.type_string.hash(state);
    }
}

/// `ChannelType::generateHash` -- a plain sum of the bytes.
pub fn generate_hash(string: &str) -> i32 {
    string.bytes().map(|byte| byte as i32).sum()
}

/// Registry of channel types, mirroring how `scripting/` hands `ChannelType *`
/// out to the language rules.
///
/// The original allocates each type once and shares the pointer; interning by
/// name reproduces that (a type is never redefined with a different parent).
#[derive(Debug, Default)]
pub struct ChannelTypeRegistry {
    types: Vec<Arc<ChannelType>>,
}

impl ChannelTypeRegistry {
    pub fn new() -> Self {
        ChannelTypeRegistry::default()
    }

    pub fn get(&self, name: &str) -> Option<Arc<ChannelType>> {
        self.types.iter().find(|t| t.type_string() == name).cloned()
    }

    /// Registers (or returns the existing) type. The parent has to be
    /// registered first, exactly like the original's static initialisation.
    pub fn register(&mut self, name: &str, parent: Option<&str>) -> Arc<ChannelType> {
        if let Some(existing) = self.get(name) {
            return existing;
        }

        let parent = parent.and_then(|parent| self.get(parent));
        let channel_type = ChannelType::with_parent(name, parent);
        self.types.push(channel_type.clone());

        channel_type
    }

    pub fn names(&self) -> Vec<&str> {
        self.types.iter().map(|t| t.type_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_byte_sums() {
        assert_eq!(generate_hash(""), 0);
        assert_eq!(generate_hash("A"), 65);
        assert_eq!(generate_hash("AB"), 131);
    }

    #[test]
    fn equality_uses_the_hash_then_the_string() {
        let a = ChannelType::new_root("float");
        let b = ChannelType::new_root("float");
        let c = ChannelType::new_root("int");

        assert!(a.is_equal(&b));
        assert!(!a.is_equal(&c));
        // "ab" and "ba" share a byte sum; the string comparison must split them.
        assert_eq!(generate_hash("ab"), generate_hash("ba"));
        assert!(!ChannelType::new_root("ab").is_equal(&ChannelType::new_root("ba")));
    }

    #[test]
    fn compatibility_walks_up_the_parent_chain() {
        let mut registry = ChannelTypeRegistry::new();
        let number = registry.register("number", None);
        let float = registry.register("float", Some("number"));
        let int = registry.register("int", Some("number"));
        let _ = number;

        assert!(float.is_compatible_with(&float));

        // The child is compatible with its parent, never the other way round.
        assert!(float.is_compatible_with(&registry.get("number").unwrap()));
        assert!(!registry.get("number").unwrap().is_compatible_with(&float));

        // Siblings are not compatible.
        assert!(!float.is_compatible_with(&int));
        assert!(!int.is_compatible_with(&float));
    }

    #[test]
    fn chains_and_lookup() {
        let mut registry = ChannelTypeRegistry::new();
        registry.register("number", None);
        registry.register("float", Some("number"));

        let float = registry.get("float").unwrap();
        let names: Vec<&str> = float.chain().iter().map(|t| t.type_string()).collect();
        assert_eq!(names, vec!["float", "number"]);

        assert!(float.find("number").is_some());
        assert!(float.find("string").is_none());
    }

    #[test]
    fn registering_twice_returns_the_same_type() {
        let mut registry = ChannelTypeRegistry::new();
        let first = registry.register("float", None);
        let second = registry.register("float", None);

        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(registry.names(), vec!["float"]);
    }
}
