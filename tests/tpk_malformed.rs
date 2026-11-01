//! A malformed TPK file has to give an error (or no type tree) rather than panic, hang or
//! allocate for sizes and counts that the data doesn't back.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use rabex::UnityVersion;
use rabex::objects::ClassId;
use rabex::tpk::{
    TpkCommonString, TpkFile, TpkTypeTreeBlob, TpkUnityClassFlags, TpkUnityNode, UnityClass,
};

/// Records the largest allocation made on each thread.
struct TrackLargest;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn track(size: usize) {
    let _ = LARGEST.try_with(|largest| largest.set(largest.get().max(size)));
}

unsafe impl GlobalAlloc for TrackLargest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        track(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: TrackLargest = TrackLargest;

/// Asserts that `f` makes no allocation of a megabyte or more.
fn assert_small_allocations<T>(what: &str, f: impl FnOnce() -> T) -> T {
    LARGEST.set(0);
    let result = f();
    let largest = LARGEST.get();
    assert!(largest < 1 << 20, "{what}: allocated {largest} bytes");
    result
}

fn tpk_file(
    compression: u8,
    payload: &[u8],
    compressed_size: i32,
    uncompressed_size: i32,
) -> Vec<u8> {
    let mut bytes = b"TPK*".to_vec();
    bytes.extend([1, compression, 0, 0]); // version, compression, data type, reserved
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(compressed_size.to_le_bytes());
    bytes.extend(uncompressed_size.to_le_bytes());
    bytes.extend(payload);
    bytes
}

/// A type tree blob with one class, `Base base { int x }`, and one entry in every other table,
/// along with the offsets of its seven counts.
fn blob() -> (Vec<u8>, [usize; 7]) {
    let version = (2u64 << 48).to_le_bytes();
    let mut bytes = Vec::new();
    let mut counts = [0; 7];
    let mut count = |bytes: &mut Vec<u8>, i: usize, n: i32| {
        counts[i] = bytes.len();
        bytes.extend(n.to_le_bytes());
    };

    bytes.extend(0i64.to_le_bytes()); // creation time
    count(&mut bytes, 0, 1); // versions
    bytes.extend(version);
    count(&mut bytes, 1, 1); // classes
    bytes.extend(1i32.to_le_bytes());
    count(&mut bytes, 2, 1); // versions of the class
    bytes.extend(version);
    bytes.push(1);
    bytes.extend([0, 0, 0, 0, 0xC0, 0, 0, 0, 0]); // name, base, flags, both root nodes
    count(&mut bytes, 3, 1); // common string versions
    bytes.extend(version);
    bytes.push(1);
    count(&mut bytes, 4, 1); // common string indices
    bytes.extend(0u16.to_le_bytes());
    count(&mut bytes, 5, 2); // nodes
    for (type_name, name, sub_nodes) in [(0u16, 1u16, &[1u16][..]), (2, 3, &[])] {
        bytes.extend(type_name.to_le_bytes());
        bytes.extend(name.to_le_bytes());
        bytes.extend((-1i32).to_le_bytes()); // byte size
        bytes.extend(1i16.to_le_bytes()); // version
        bytes.push(0); // type flags
        bytes.extend(0u32.to_le_bytes()); // meta flag
        bytes.extend((sub_nodes.len() as u16).to_le_bytes());
        for sub_node in sub_nodes {
            bytes.extend(sub_node.to_le_bytes());
        }
    }
    count(&mut bytes, 6, 4); // strings
    for string in ["Base", "base", "int", "x"] {
        bytes.push(string.len() as u8);
        bytes.extend(string.as_bytes());
    }
    (bytes, counts)
}

fn with_count(at: usize, n: i32) -> Vec<u8> {
    let (mut bytes, counts) = blob();
    bytes[counts[at]..counts[at] + 4].copy_from_slice(&n.to_le_bytes());
    bytes
}

fn parse(bytes: &[u8]) -> Result<TpkTypeTreeBlob, rabex::tpk::Error> {
    TpkTypeTreeBlob::from_reader(&mut &bytes[..])
}

#[test]
fn negative_sizes_are_errors() {
    let (payload, _) = blob();
    let len = payload.len() as i32;

    for compressed_size in [-1, i32::MIN] {
        let file = tpk_file(0, &payload, compressed_size, len);
        assert!(TpkFile::from_reader(&mut file.as_slice()).is_err());
    }

    for compression in [1, 2, 3] {
        let file = tpk_file(compression, &payload, len, -1);
        let tpk = TpkFile::from_reader(&mut file.as_slice()).unwrap();
        assert!(tpk.as_type_tree().is_err(), "compression {compression}");
    }
}

#[test]
fn negative_counts_do_not_panic() {
    let tpk = parse(&blob().0).unwrap();
    let base = tpk
        .get_typetree_node(ClassId(1), &UnityVersion::new(2, 0, 0))
        .unwrap();
    assert_eq!(
        (base.m_Type.as_str(), base.m_Name.as_str()),
        ("Base", "base")
    );
    assert_eq!(base.children[0].m_Type, "int");

    for at in 0..7 {
        for n in [-1, i32::MIN] {
            let _ = parse(&with_count(at, n));
        }
    }
}

#[test]
fn huge_compressed_size_is_not_allocated() {
    let (payload, _) = blob();
    let file = tpk_file(0, &payload, i32::MAX, payload.len() as i32);
    let tpk = assert_small_allocations("compressed size", || {
        TpkFile::from_reader(&mut file.as_slice())
    });
    assert!(tpk.is_err());
}

#[test]
fn huge_counts_are_not_allocated() {
    for at in 0..7 {
        let bytes = with_count(at, i32::MAX);
        let blob = assert_small_allocations(&format!("count {at}"), || parse(&bytes));
        assert!(blob.is_err(), "count {at}");
    }
}

#[test]
fn huge_string_length_is_not_allocated() {
    // one string, whose length is the largest 7-bit encoded int (2^35 - 1)
    let (mut bytes, counts) = blob();
    bytes.truncate(counts[6]);
    bytes.extend(1i32.to_le_bytes());
    bytes.extend([0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
    bytes.extend(b"Base");
    let blob = assert_small_allocations("string length", || parse(&bytes));
    assert!(blob.is_err());
}

#[cfg(feature = "compression-lz4")]
#[test]
fn lz4_size_beyond_the_block_is_an_error() {
    let (payload, _) = blob();
    let block = lz4_flex::block::compress(&payload);

    let file = tpk_file(1, &block, block.len() as i32, payload.len() as i32);
    let tpk = TpkFile::from_reader(&mut file.as_slice()).unwrap();
    assert!(tpk.as_type_tree().unwrap().is_some());

    // a block expands to at most 255 times its size
    let max = block.len() as i32 * 255;
    let file = tpk_file(1, &block, block.len() as i32, max);
    let tpk = TpkFile::from_reader(&mut file.as_slice()).unwrap();
    assert!(tpk.as_type_tree().unwrap().is_some());

    for uncompressed_size in [max + 1, i32::MAX] {
        let file = tpk_file(1, &block, block.len() as i32, uncompressed_size);
        let tpk = TpkFile::from_reader(&mut file.as_slice()).unwrap();
        assert!(tpk.as_type_tree().is_err(), "{uncompressed_size}");
    }
}

#[test]
fn common_string_versions_out_of_order() {
    // the versions 5.0.0, 3.0.0 and 7.0.0, with 3, 1 and 9 strings
    let mut bytes = Vec::new();
    bytes.extend(3i32.to_le_bytes());
    for (major, count) in [(5u64, 3), (3, 1), (7, 9)] {
        bytes.extend((major << 48).to_le_bytes());
        bytes.push(count);
    }
    bytes.extend(0i32.to_le_bytes()); // string buffer indices
    let common_string = TpkCommonString::read(&mut bytes.as_slice()).unwrap();

    let version = |major| UnityVersion::new(major, 0, 0);
    assert_eq!(common_string.get_count(&version(4)), None);
    assert_eq!(common_string.get_count(&version(6)), Some(1));
}

/// A blob of nodes where node `i` lists `sub_nodes[i]`, and a class rooted at node 0.
fn tree(sub_nodes: Vec<Vec<u16>>) -> (TpkTypeTreeBlob, UnityClass) {
    let blob = TpkTypeTreeBlob {
        creation_time: 0,
        versions: Vec::new(),
        class_information: Default::default(),
        common_string: TpkCommonString {
            version_information: Vec::new(),
            string_buffer_indices: Vec::new(),
        },
        node_buffer: sub_nodes
            .into_iter()
            .map(|sub_nodes| TpkUnityNode {
                type_name: 0,
                name: 1,
                byte_size: -1,
                version: 1,
                type_flags: 0,
                meta_flag: 0,
                sub_nodes,
            })
            .collect(),
        string_buffer: vec!["Base".into(), "base".into()],
    };
    let class = UnityClass {
        name: 0,
        base: 0,
        flags: TpkUnityClassFlags::HAS_RELEASE_ROOT_NODE,
        editor_root_node: None,
        release_root_node: Some(0),
    };
    (blob, class)
}

#[test]
fn node_or_string_out_of_range_has_no_typetree() {
    let (blob, mut class) = tree(vec![vec![1], vec![]]);
    assert!(blob.get_typetree_node_for_class(&class, false).is_some());

    class.release_root_node = Some(2);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());

    let (blob, class) = tree(vec![vec![2], vec![]]);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());

    let (mut blob, class) = tree(vec![vec![1], vec![]]);
    blob.node_buffer[1].name = 2;
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());
}

#[test]
fn cyclic_node_tree_has_no_typetree() {
    // node 0 lists itself
    let (blob, class) = tree(vec![vec![0]]);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());

    // node 2 lists node 0, its grandparent
    let (blob, class) = tree(vec![vec![1], vec![2], vec![0]]);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());
}

#[test]
fn shared_nodes_cannot_build_an_exponential_typetree() {
    // node `i` lists node `i + 1` twice, so the tree has 2^17 - 1 nodes
    let mut sub_nodes: Vec<_> = (1..17).map(|i| vec![i, i]).collect();
    sub_nodes.push(Vec::new());
    let (blob, class) = tree(sub_nodes);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());
}

#[test]
fn long_names_shared_by_many_nodes_have_no_typetree() {
    // node 0 lists node 1 20,000 times
    let (mut blob, class) = tree(vec![vec![1; 20_000], vec![]]);
    assert!(blob.get_typetree_node_for_class(&class, false).is_some());

    // with a 2 KiB name, which every node copies: 39 MiB in all
    blob.string_buffer[1] = "x".repeat(2048);
    assert!(blob.get_typetree_node_for_class(&class, false).is_none());
}

#[test]
fn node_tree_deeper_than_256_levels() {
    // node `i` lists node `i + 1`, 300 levels deep
    let mut sub_nodes: Vec<_> = (1..300).map(|i| vec![i]).collect();
    sub_nodes.push(Vec::new());
    let (blob, class) = tree(sub_nodes);

    let mut node = &blob.get_typetree_node_for_class(&class, false).unwrap();
    let mut depth = 1;
    while let Some(child) = node.children.first() {
        node = child;
        depth += 1;
    }
    assert_eq!(depth, 300);
}
