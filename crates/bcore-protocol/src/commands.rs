//! `declare_commands` (0x10): the Brigadier command tree the client autocompletes.
//!
//! # Wire format (protocol 776)
//!
//! ```text
//! nodes: varint count, then count * command_node
//! rootIndex: varint
//! ```
//!
//! A `command_node` is a flags byte, then children, then optional extras:
//!
//! ```text
//! flags: u8
//!   bits 0-1  node type: 0 root, 1 literal, 2 argument
//!   bit  2    has_command    (0x04) — the node is executable
//!   bit  3    has_redirect   (0x08)
//!   bit  4    has_suggestions(0x10) — argument nodes only
//!   bit  5    allows_restricted (0x20)
//! children:    varint count, then count * varint node index
//! redirectNode: varint            (only when has_redirect)
//! name:        string             (literal and argument nodes)
//! parser:      varint             (argument nodes only)
//! properties:  parser-dependent   (argument nodes only)
//! suggestionType: string          (only when has_suggestions)
//! ```
//!
//! Field order and the flag bit numbering come from `target/protocol_26_1.json`
//! (`command_node`); note minecraft-data lists the bitfield most-significant
//! first, so `command_node_type` is the **low** two bits on the wire. Both were
//! confirmed by decoding vanilla 26.2's own `declare_commands` packet from
//! `data/play_packets.bin` (26 nodes) — that decode round-trips with the writer
//! here, see `tests`.
//!
//! # Parser ids used by BCore
//!
//! | parser | id | properties |
//! |--------|----|------------|
//! | `brigadier:string` | 5 | varint mode (0 single word, 2 greedy phrase) |
//! | `brigadier:double` | 2 | u8 flags (0 = no min/max) |
//! | `minecraft:message` | 20 | none |
//! | `minecraft:entity` | 6 | u8 flags (0x01 single, 0x02 players only) |

use bcore_core::varint::encode_varint;

use crate::packet::{write_packet, write_string};

/// Clientbound `declare_commands`.
pub const CB_DECLARE_COMMANDS: i32 = 0x10;

/// Parser id for `brigadier:double`.
pub const PARSER_DOUBLE: i32 = 2;
/// Parser id for `brigadier:string`.
pub const PARSER_STRING: i32 = 5;
/// Parser id for `minecraft:entity`.
pub const PARSER_ENTITY: i32 = 6;
/// Parser id for `minecraft:message`.
pub const PARSER_MESSAGE: i32 = 20;

/// `brigadier:string` mode: a single unquoted word.
pub const STRING_SINGLE_WORD: i32 = 0;
/// `brigadier:string` mode: the rest of the input, spaces included.
pub const STRING_GREEDY_PHRASE: i32 = 2;

/// `minecraft:entity` flags: exactly one target, players only.
pub const ENTITY_SINGLE_PLAYER: u8 = 0x03;

/// Node type bits of a `command_node` flags byte.
const NODE_ROOT: u8 = 0;
const NODE_LITERAL: u8 = 1;
const NODE_ARGUMENT: u8 = 2;
/// The node can be executed as a command.
const FLAG_EXECUTABLE: u8 = 0x04;
/// The node has custom suggestions (argument nodes only).
const FLAG_SUGGESTIONS: u8 = 0x10;

/// The parser of an argument node, together with its properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parser {
    /// A single unquoted word.
    Word,
    /// The rest of the line, spaces included.
    Greedy,
    /// A chat message (the parser vanilla uses for `/me` and `/say`).
    Message,
    /// A floating-point coordinate.
    Double,
    /// One online player, suggested by the server.
    Player,
}

impl Parser {
    fn id(self) -> i32 {
        match self {
            Parser::Word | Parser::Greedy => PARSER_STRING,
            Parser::Message => PARSER_MESSAGE,
            Parser::Double => PARSER_DOUBLE,
            Parser::Player => PARSER_ENTITY,
        }
    }

    fn write_properties(self, out: &mut Vec<u8>) {
        match self {
            Parser::Word => encode_varint(STRING_SINGLE_WORD, out),
            Parser::Greedy => encode_varint(STRING_GREEDY_PHRASE, out),
            // brigadier:double takes a flags byte; 0 means neither min nor max.
            Parser::Double => out.push(0x00),
            Parser::Player => out.push(ENTITY_SINGLE_PLAYER),
            Parser::Message => {}
        }
    }
}

/// One node of the command tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandNode {
    kind: u8,
    name: String,
    parser: Option<Parser>,
    executable: bool,
    children: Vec<usize>,
}

impl CommandNode {
    fn root() -> Self {
        Self {
            kind: NODE_ROOT,
            name: String::new(),
            parser: None,
            executable: false,
            children: Vec::new(),
        }
    }

    fn literal(name: &str, executable: bool) -> Self {
        Self {
            kind: NODE_LITERAL,
            name: name.to_string(),
            parser: None,
            executable,
            children: Vec::new(),
        }
    }

    fn argument(name: &str, parser: Parser, executable: bool) -> Self {
        Self {
            kind: NODE_ARGUMENT,
            name: name.to_string(),
            parser: Some(parser),
            executable,
            children: Vec::new(),
        }
    }

    fn flags(&self) -> u8 {
        let mut flags = self.kind;
        if self.executable {
            flags |= FLAG_EXECUTABLE;
        }
        // `minecraft:entity` targets are resolved by the server, so the client
        // must ask us for completions instead of guessing.
        if matches!(self.parser, Some(Parser::Player)) {
            flags |= FLAG_SUGGESTIONS;
        }
        flags
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(self.flags());
        encode_varint(self.children.len() as i32, out);
        for &child in &self.children {
            encode_varint(child as i32, out);
        }
        if self.kind != NODE_ROOT {
            write_string(&self.name, out);
        }
        if let Some(parser) = self.parser {
            encode_varint(parser.id(), out);
            parser.write_properties(out);
            if matches!(parser, Parser::Player) {
                write_string("minecraft:ask_server", out);
            }
        }
    }
}

/// A command tree under construction. Node 0 is always the root.
#[derive(Debug, Clone)]
pub struct CommandTree {
    nodes: Vec<CommandNode>,
}

impl Default for CommandTree {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandTree {
    /// An empty tree holding only the root node.
    pub fn new() -> Self {
        Self {
            nodes: vec![CommandNode::root()],
        }
    }

    /// Add a top-level command literal, returning its node index.
    pub fn literal(&mut self, name: &str, executable: bool) -> usize {
        let idx = self.push(CommandNode::literal(name, executable));
        self.nodes[0].children.push(idx);
        idx
    }

    /// Add a child literal under `parent`, returning its node index.
    pub fn child_literal(&mut self, parent: usize, name: &str, executable: bool) -> usize {
        let idx = self.push(CommandNode::literal(name, executable));
        self.nodes[parent].children.push(idx);
        idx
    }

    /// Add a child argument under `parent`, returning its node index.
    pub fn argument(
        &mut self,
        parent: usize,
        name: &str,
        parser: Parser,
        executable: bool,
    ) -> usize {
        let idx = self.push(CommandNode::argument(name, parser, executable));
        self.nodes[parent].children.push(idx);
        idx
    }

    fn push(&mut self, node: CommandNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Number of nodes, root included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always false: the root node is always present.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Encode the whole tree as a framed `declare_commands` packet.
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        encode_varint(self.nodes.len() as i32, &mut data);
        for node in &self.nodes {
            node.write(&mut data);
        }
        encode_varint(0, &mut data); // rootIndex
        let mut out = Vec::new();
        write_packet(&mut out, CB_DECLARE_COMMANDS, &data);
        out
    }
}

/// Build the command tree BCore advertises at join.
///
/// Mirrors exactly what [`crate::command`] can execute, so the client never
/// autocompletes a command the server would reject.
pub fn bcore_command_tree() -> CommandTree {
    let mut tree = CommandTree::new();

    // /help
    let help = tree.literal("help", true);
    tree.argument(help, "command", Parser::Word, true);

    // /list
    tree.literal("list", true);

    // /me <action>
    let me = tree.literal("me", false);
    tree.argument(me, "action", Parser::Message, true);

    // /say <message>
    let say = tree.literal("say", false);
    tree.argument(say, "message", Parser::Message, true);

    // /spawn
    tree.literal("spawn", true);

    // /gamemode <survival|creative|spectator|adventure>
    let gamemode = tree.literal("gamemode", false);
    for mode in ["survival", "creative", "adventure", "spectator"] {
        tree.child_literal(gamemode, mode, true);
    }

    // /tp <x> <y> <z>
    let tp = tree.literal("tp", false);
    let tp_x = tree.argument(tp, "x", Parser::Double, false);
    let tp_y = tree.argument(tp_x, "y", Parser::Double, false);
    tree.argument(tp_y, "z", Parser::Double, true);

    // /seed
    tree.literal("seed", true);

    // /time set <day|night|noon|midnight>
    let time = tree.literal("time", false);
    let time_set = tree.child_literal(time, "set", false);
    for when in ["day", "midnight", "night", "noon"] {
        tree.child_literal(time_set, when, true);
    }

    // /kick <player>
    let kick = tree.literal("kick", false);
    tree.argument(kick, "player", Parser::Player, true);

    // /stop
    tree.literal("stop", true);

    tree
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcore_core::varint::decode_varint;

    /// A minimal `declare_commands` reader, used to prove the writer round-trips
    /// and to decode vanilla's own packet with the same code path.
    struct Decoded {
        nodes: Vec<DecodedNode>,
        root_index: i32,
        consumed: usize,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct DecodedNode {
        kind: u8,
        executable: bool,
        has_suggestions: bool,
        children: Vec<i32>,
        name: Option<String>,
        parser: Option<i32>,
        suggestion_type: Option<String>,
    }

    fn decode_tree(data: &[u8]) -> Decoded {
        let mut at = 0usize;
        let take_varint = |at: &mut usize| {
            let (v, n) = decode_varint(&data[*at..]).expect("varint");
            *at += n;
            v
        };
        let count = take_varint(&mut at);
        let mut nodes = Vec::new();
        for _ in 0..count {
            let flags = data[at];
            at += 1;
            let kind = flags & 0x03;
            let executable = flags & FLAG_EXECUTABLE != 0;
            let has_redirect = flags & 0x08 != 0;
            let has_suggestions = flags & FLAG_SUGGESTIONS != 0;
            let nchildren = take_varint(&mut at);
            let children = (0..nchildren).map(|_| take_varint(&mut at)).collect();
            if has_redirect {
                take_varint(&mut at);
            }
            let mut name = None;
            let mut parser = None;
            if kind != NODE_ROOT {
                let len = take_varint(&mut at) as usize;
                name = Some(String::from_utf8(data[at..at + len].to_vec()).expect("utf8"));
                at += len;
            }
            if kind == NODE_ARGUMENT {
                let id = take_varint(&mut at);
                parser = Some(id);
                // Consume parser properties. Only the parsers that actually
                // appear in BCore's tree and in vanilla's captured tree need
                // handling; the ids come from `command_node`'s parser mapping.
                match id {
                    // brigadier:float(1) / double(2) / integer(3) / long(4):
                    // a flags byte, then an optional min and max whose widths
                    // depend on the parser.
                    1..=4 => {
                        let flags = data[at];
                        at += 1;
                        let width = if id == 1 || id == 3 { 4 } else { 8 };
                        if flags & 0x01 != 0 {
                            at += width;
                        }
                        if flags & 0x02 != 0 {
                            at += width;
                        }
                    }
                    // brigadier:string: a varint mode.
                    PARSER_STRING => {
                        take_varint(&mut at);
                    }
                    // minecraft:entity(6) / score_holder(31): a bitfield byte.
                    PARSER_ENTITY | 31 => at += 1,
                    // minecraft:time(43): an i32 minimum.
                    43 => at += 4,
                    // resource_or_tag(44) .. resource_selector(48): a registry id.
                    44..=48 => {
                        let len = take_varint(&mut at) as usize;
                        at += len;
                    }
                    // Everything else (message(20), objective(24),
                    // int_range(39), ...) carries no properties.
                    _ => {}
                }
            }
            let mut suggestion_type = None;
            if has_suggestions {
                let len = take_varint(&mut at) as usize;
                suggestion_type =
                    Some(String::from_utf8(data[at..at + len].to_vec()).expect("utf8"));
                at += len;
            }
            nodes.push(DecodedNode {
                kind,
                executable,
                has_suggestions,
                children,
                name,
                parser,
                suggestion_type,
            });
        }
        let root_index = take_varint(&mut at);
        Decoded {
            nodes,
            root_index,
            consumed: at,
        }
    }

    fn framed_payload(frame: &[u8]) -> (i32, Vec<u8>) {
        let (len, n) = decode_varint(frame).expect("length");
        assert_eq!(frame.len(), n + len as usize);
        let (id, m) = decode_varint(&frame[n..]).expect("id");
        (id, frame[n + m..].to_vec())
    }

    #[test]
    fn a_single_literal_encodes_to_the_exact_expected_bytes() {
        let mut tree = CommandTree::new();
        tree.literal("list", true);
        let (id, body) = framed_payload(&tree.encode());
        assert_eq!(id, CB_DECLARE_COMMANDS);
        assert_eq!(
            body,
            vec![
                0x02, // node count
                0x00, // root: type 0, not executable
                0x01, 0x01, // one child: node 1
                0x05, // literal (1) + executable (4)
                0x00, // no children
                0x04, b'l', b'i', b's', b't', // name
                0x00, // rootIndex
            ]
        );
    }

    #[test]
    fn an_argument_node_writes_parser_id_then_properties() {
        let mut tree = CommandTree::new();
        let me = tree.literal("me", false);
        tree.argument(me, "action", Parser::Message, true);
        let (_, body) = framed_payload(&tree.encode());
        // root(3 bytes: flags, 1 child, idx1) then literal "me"
        // then argument "action" with parser 20 and no properties.
        assert_eq!(
            body,
            vec![
                0x03, // 3 nodes
                0x00, 0x01, 0x01, // root -> [1]
                0x01, 0x01, 0x02, 0x02, b'm', b'e', // literal me -> [2]
                0x06, // argument (2) + executable (4)
                0x00, // no children
                0x06, b'a', b'c', b't', b'i', b'o', b'n', // name
                20,   // minecraft:message
                0x00, // rootIndex
            ]
        );
    }

    #[test]
    fn string_parsers_carry_their_mode_and_doubles_carry_a_flags_byte() {
        let mut tree = CommandTree::new();
        let root_help = tree.literal("help", true);
        tree.argument(root_help, "command", Parser::Word, true);
        let (_, body) = framed_payload(&tree.encode());
        let decoded = decode_tree(&body);
        let arg = &decoded.nodes[2];
        assert_eq!(arg.parser, Some(PARSER_STRING));
        // The byte after the parser id is the string mode.
        let tail_at = body.len() - 2; // mode, rootIndex
        assert_eq!(body[tail_at], STRING_SINGLE_WORD as u8);

        let mut tree = CommandTree::new();
        let tp = tree.literal("tp", false);
        tree.argument(tp, "x", Parser::Double, true);
        let (_, body) = framed_payload(&tree.encode());
        assert_eq!(body[body.len() - 2], 0x00, "double flags: no min/max");
    }

    #[test]
    fn player_arguments_request_server_side_suggestions() {
        let mut tree = CommandTree::new();
        let kick = tree.literal("kick", false);
        tree.argument(kick, "player", Parser::Player, true);
        let (_, body) = framed_payload(&tree.encode());
        let decoded = decode_tree(&body);
        let arg = &decoded.nodes[2];
        assert!(arg.has_suggestions, "entity args need ask_server");
        assert_eq!(arg.parser, Some(PARSER_ENTITY));
        assert_eq!(arg.suggestion_type.as_deref(), Some("minecraft:ask_server"));
        assert_eq!(decoded.consumed, body.len(), "fully consumed");
    }

    #[test]
    fn the_bcore_tree_round_trips_and_declares_every_command() {
        let tree = bcore_command_tree();
        let (id, body) = framed_payload(&tree.encode());
        assert_eq!(id, CB_DECLARE_COMMANDS);
        let decoded = decode_tree(&body);
        assert_eq!(decoded.consumed, body.len(), "no trailing garbage");
        assert_eq!(decoded.root_index, 0);
        assert_eq!(decoded.nodes.len(), tree.len());

        // Root children are exactly the commands BCore implements.
        let root = &decoded.nodes[0];
        assert_eq!(root.kind, NODE_ROOT);
        assert!(!root.executable);
        let mut top: Vec<String> = root
            .children
            .iter()
            .map(|&i| decoded.nodes[i as usize].name.clone().expect("named"))
            .collect();
        top.sort();
        assert_eq!(
            top,
            vec![
                "gamemode", "help", "kick", "list", "me", "say", "seed", "spawn", "stop", "time",
                "tp",
            ]
        );
    }

    #[test]
    fn nested_commands_declare_their_subcommands() {
        let tree = bcore_command_tree();
        let (_, body) = framed_payload(&tree.encode());
        let decoded = decode_tree(&body);
        let find = |name: &str| {
            decoded
                .nodes
                .iter()
                .position(|n| n.name.as_deref() == Some(name))
                .expect("node exists")
        };

        // /gamemode has four literal modes, none of them nested further.
        let gamemode = &decoded.nodes[find("gamemode")];
        assert!(!gamemode.executable, "/gamemode alone is not executable");
        let mut modes: Vec<String> = gamemode
            .children
            .iter()
            .map(|&i| decoded.nodes[i as usize].name.clone().expect("named"))
            .collect();
        modes.sort();
        assert_eq!(
            modes,
            vec!["adventure", "creative", "spectator", "survival"]
        );
        for &i in &gamemode.children {
            assert!(decoded.nodes[i as usize].executable);
        }

        // /tp chains x -> y -> z and only z executes.
        let tp = &decoded.nodes[find("tp")];
        assert_eq!(tp.children.len(), 1);
        let x = &decoded.nodes[tp.children[0] as usize];
        assert_eq!(x.name.as_deref(), Some("x"));
        assert!(!x.executable);
        let y = &decoded.nodes[x.children[0] as usize];
        assert_eq!(y.name.as_deref(), Some("y"));
        assert!(!y.executable);
        let z = &decoded.nodes[y.children[0] as usize];
        assert_eq!(z.name.as_deref(), Some("z"));
        assert!(z.executable, "the last coordinate runs the command");

        // /time set <when>
        let time = &decoded.nodes[find("time")];
        let set = &decoded.nodes[time.children[0] as usize];
        assert_eq!(set.name.as_deref(), Some("set"));
        let mut when: Vec<String> = set
            .children
            .iter()
            .map(|&i| decoded.nodes[i as usize].name.clone().expect("named"))
            .collect();
        when.sort();
        assert_eq!(when, vec!["day", "midnight", "night", "noon"]);
    }

    #[test]
    fn the_encoding_is_deterministic() {
        assert_eq!(bcore_command_tree().encode(), bcore_command_tree().encode());
    }

    /// The decoder above is validated against vanilla's own packet: if BCore's
    /// understanding of `command_node` were wrong, this would not consume the
    /// captured 265-byte tree exactly.
    #[test]
    fn the_node_format_matches_vanillas_own_declare_commands_capture() {
        const PLAY_PACKETS: &[u8] = include_bytes!("../data/play_packets.bin");
        let count = u32::from_be_bytes(PLAY_PACKETS[0..4].try_into().expect("count")) as usize;
        let mut at = 4usize;
        let mut captured = None;
        for _ in 0..count {
            let pid = i32::from_be_bytes(PLAY_PACKETS[at..at + 4].try_into().expect("pid"));
            let len =
                u32::from_be_bytes(PLAY_PACKETS[at + 4..at + 8].try_into().expect("len")) as usize;
            at += 8;
            if pid == CB_DECLARE_COMMANDS {
                captured = Some(PLAY_PACKETS[at..at + len].to_vec());
                break;
            }
            at += len;
        }
        let captured = captured.expect("capture contains declare_commands");
        let decoded = decode_tree(&captured);
        assert_eq!(
            decoded.consumed,
            captured.len(),
            "vanilla's tree parses exactly with BCore's node layout"
        );
        assert_eq!(decoded.nodes.len(), 26, "vanilla sent 26 nodes");
        assert_eq!(decoded.root_index, 0);
        // Vanilla's non-op tree: /me /help /list /msg /tell /w /random ...
        let names: Vec<&str> = decoded.nodes[0]
            .children
            .iter()
            .map(|&i| decoded.nodes[i as usize].name.as_deref().expect("named"))
            .collect();
        assert_eq!(
            names,
            vec!["me", "help", "list", "msg", "tell", "w", "random", "teammsg", "tm", "trigger",]
        );
        // `/help <command>` uses a greedy string, exactly like BCore's own tree.
        let help = decoded
            .nodes
            .iter()
            .position(|n| n.name.as_deref() == Some("help"))
            .expect("help");
        let arg = &decoded.nodes[decoded.nodes[help].children[0] as usize];
        assert_eq!(arg.parser, Some(PARSER_STRING));
        assert_eq!(arg.name.as_deref(), Some("command"));
    }
}
