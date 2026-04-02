use crate::argus_json::{self, JsonValue};
use std::cell::RefCell;
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;
use std::hash::Hash;
use std::num::ParseIntError;
use std::rc::Rc;

thread_local! {
static NODES: RefCell<Vec<Option<Rc<Node>>>> = const { RefCell::new(Vec::new()) };
}

fn get_nodes_len() -> usize {
    NODES.with(|nodes_ref| nodes_ref.borrow().len())
}

fn get_node(id: usize) -> Option<Option<Rc<Node>>> {
    NODES.with(|nodes_ref| nodes_ref.borrow().get(id).cloned())
}

fn push_node(node: Node) {
    NODES.with(|nodes_ref| nodes_ref.borrow_mut().push(Some(Rc::new(node))));
}

fn null_out_node(id: usize) {
    NODES.with(|nodes_ref| {
        let mut nodes = nodes_ref.borrow_mut();
        if let Some(slot) = nodes.get_mut(id) {
            *slot = None;
        }
    });
}

// Container for a node. Exist mainly to be able to implement Hash, which is not implemented for RefCell
#[derive(Default, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Node {
    // A unique id that acts as its index in NODES
    unique_id: usize,
    // Position of the node in byond
    x: usize,
    y: usize,
    z: usize,
    // Indexes of nodes connected to this one
    connected_nodes_id: Vec<usize>,
}

impl Node {
    /// Parse a single Node from a JsonValue object.
    fn from_json(val: &JsonValue) -> Result<Self, ()> {
        let unique_id = val.get("unique_id").and_then(|v| v.as_i64()).ok_or(())? as usize;
        let x = val.get("x").and_then(|v| v.as_i64()).ok_or(())? as usize;
        let y = val.get("y").and_then(|v| v.as_i64()).ok_or(())? as usize;
        let z = val.get("z").and_then(|v| v.as_i64()).ok_or(())? as usize;
        let connected_arr = val.get("connected_nodes_id").and_then(|v| v.as_array()).ok_or(())?;
        let connected_nodes_id: Vec<usize> = connected_arr
            .iter()
            .map(|v| v.as_i64().map(|n| n as usize).ok_or(()))
            .collect::<Result<Vec<_>, ()>>()?;
        Ok(Node {
            unique_id,
            x,
            y,
            z,
            connected_nodes_id,
        })
    }

    /// Serialize a Node to a JSON string.
    fn to_json_string(&self) -> String {
        let connected: String = self
            .connected_nodes_id
            .iter()
            .enumerate()
            .fold(String::new(), |mut acc, (i, id)| {
                if i > 0 {
                    acc.push(',');
                }
                use std::fmt::Write;
                let _ = write!(acc, "{}", id);
                acc
            });
        format!(
            "{{\"unique_id\":{},\"x\":{},\"y\":{},\"z\":{},\"connected_nodes_id\":[{}]}}",
            self.unique_id, self.x, self.y, self.z, connected
        )
    }

    // Return a vector of all connected nodes, encapsulated in a NodeContainer.
    fn successors(&self) -> Vec<(Rc<Node>, usize)> {
        self.connected_nodes_id
            .iter()
            .filter_map(|index| get_node(*index))
            .flatten()
            .map(|node| (node.clone(), self.distance(node.as_ref())))
            .collect()
    }

    // Return the geometric distance between this node and another one.
    fn distance(&self, other: &Self) -> usize {
        (((self.x as isize - other.x as isize).pow(2)
            + (self.y as isize - other.y as isize).pow(2)) as usize)
            .isqrt()
    }
}

#[derive(Debug)]
enum RegisteringNodesError {
    ParseError,
    NodesNotCorrectlyIndexed,
}

impl std::fmt::Display for RegisteringNodesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseError => write!(f, "Invalid JSON input"),
            Self::NodesNotCorrectlyIndexed => write!(f, "Nodes were not correctly indexed"),
        }
    }
}

impl From<()> for RegisteringNodesError {
    fn from(_: ()) -> Self {
        RegisteringNodesError::ParseError
    }
}

byond_fn!(fn clear_nodes_astar() {
    NODES.with(|nodes_ref| nodes_ref.borrow_mut().clear());
    Some("1".to_string())
});

byond_fn!(fn register_nodes_astar(json) {
    match register_nodes(json) { Ok(s) => Some(s),
        Err(e) => Some(format!("{e}"))
    }
});

// Builds a list of nodes from a json file.
// Errors if the input list of nodes is not correctly indexed. Each node should have for unique id its position in the list, with the first unique-id being 0.
fn register_nodes(json: &str) -> Result<String, RegisteringNodesError> {
    let parsed = argus_json::parse_value(json.as_bytes())?;
    let arr = parsed.as_array().ok_or(RegisteringNodesError::ParseError)?;
    let deserialized_nodes: Vec<Node> = arr
        .iter()
        .map(Node::from_json)
        .collect::<Result<Vec<_>, ()>>()?;

    if deserialized_nodes
        .iter()
        .enumerate()
        .filter(|(i, node)| i != &node.unique_id)
        .count()
        != 0
    {
        return Err(RegisteringNodesError::NodesNotCorrectlyIndexed);
    }

    deserialized_nodes.into_iter().for_each(push_node);

    Ok("1".to_string())
}

byond_fn!(fn add_node_astar(json) {
    match add_node(json) {
        Ok(s) => Some(s),
        Err(e) => Some(format!("{e}"))
    }
});

// Add a node to the static list of node.
// If it is connected to other existing nodes, it will update their connected_nodes_id list.
fn add_node(json: &str) -> Result<String, RegisteringNodesError> {
    let parsed = argus_json::parse_value(json.as_bytes())?;
    let new_node = Node::from_json(&parsed)?;

    // As always, a node unique id should correspond to its index in NODES
    if new_node.unique_id != get_nodes_len() {
        return Err(RegisteringNodesError::NodesNotCorrectlyIndexed);
    }

    // Make sure every connection we have with other nodes is 2 ways
    for index in new_node.connected_nodes_id.iter() {
        NODES.with(|nodes_ref| {
            if let Some(Some(node)) = nodes_ref.borrow_mut().get_mut(*index) {
                if let Some(inner) = Rc::get_mut(node) {
                    inner.connected_nodes_id.push(new_node.unique_id);
                }
            }
        })
    }

    push_node(new_node);

    Ok("1".to_string())
}

#[derive(Debug)]
enum DeleteNodeError {
    ParsingError(ParseIntError),
    NodeNotFound,
}

impl std::fmt::Display for DeleteNodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParsingError(e) => write!(f, "{e}"),
            Self::NodeNotFound => write!(f, "Node was not found"),
        }
    }
}

byond_fn!(fn remove_node_astar(unique_id) {
    match remove_node(unique_id) {
        Ok(s) => Some(s),
        Err(e) => Some(format!("{e}"))
    }
});

// Replace the node with unique_id by None
// Update connected nodes as well so nothing target the removed node anymore
// Errors if no node can be found with unique_id
fn remove_node(unique_id: &str) -> Result<String, DeleteNodeError> {
    let unique_id = match unique_id.parse::<usize>() {
        Ok(id) => id,
        Err(e) => return Err(DeleteNodeError::ParsingError(e)),
    };

    let node_to_delete = match get_node(unique_id) {
        Some(Some(node)) => node,
        _ => return Err(DeleteNodeError::NodeNotFound),
    };

    for index in node_to_delete.connected_nodes_id.iter() {
        NODES.with(|nodes_ref| {
            if let Some(Some(node)) = nodes_ref.borrow_mut().get_mut(*index) {
                if let Some(inner) = Rc::get_mut(node) {
                    inner.connected_nodes_id.retain(|index| index != &node_to_delete.unique_id);
                }
            }
        })
    }

    null_out_node(unique_id);

    Ok("1".to_string())
}

#[derive(Debug)]
enum AstarError {
    StartNodeNotFound,
    GoalNodeNotFound,
    NoPath,
}

impl std::fmt::Display for AstarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartNodeNotFound => write!(f, "Starting node not found"),
            Self::GoalNodeNotFound => write!(f, "Goal node not found"),
            Self::NoPath => write!(f, "No path found"),
        }
    }
}

byond_fn!(fn generate_path_astar(start_node_id, goal_node_id) {
    if let (Ok(start_node_id), Ok(goal_node_id)) = (start_node_id.parse::<usize>(), goal_node_id.parse::<usize>()) {
        match generate_path(start_node_id, goal_node_id) {
            Ok(vector) => {
                // Serialize Vec<usize> as JSON array
                let arr: Vec<JsonValue> = vector.iter().map(|&id| JsonValue::from(id)).collect();
                Some(argus_json::serialize_value(&JsonValue::Array(arr)))
            },
            Err(e) => Some(format!("{e}"))
        }
    }
    else {
        Some("Invalid arguments".to_string())
    }
});

// Compute the shortest path between start node and goal node using A*
fn generate_path(start_node_id: usize, goal_node_id: usize) -> Result<Vec<usize>, AstarError> {
    let start_node = match get_node(start_node_id) {
        Some(Some(node)) => node,
        _ => return Err(AstarError::StartNodeNotFound),
    };

    let goal_node = match get_node(goal_node_id) {
        Some(Some(node)) => node,
        _ => return Err(AstarError::GoalNodeNotFound),
    };

    if goal_node.z != start_node.z {
        return Err(AstarError::NoPath);
    }

    // A* search
    // Priority queue: (Reverse(f_score), unique_id) — Reverse for min-heap
    let mut open: BinaryHeap<(Reverse<usize>, usize)> = BinaryHeap::new();
    let mut g_score: HashMap<usize, usize> = HashMap::new();
    let mut came_from: HashMap<usize, usize> = HashMap::new();

    g_score.insert(start_node.unique_id, 0);
    open.push((Reverse(start_node.distance(&goal_node)), start_node.unique_id));

    while let Some((_, current_id)) = open.pop() {
        if current_id == goal_node.unique_id {
            // Reconstruct path
            let mut path = vec![current_id];
            let mut id = current_id;
            while let Some(&prev) = came_from.get(&id) {
                path.push(prev);
                id = prev;
            }
            // path is already goal→start, which is the reversed order BYOND wants
            return Ok(path);
        }

        let current = match get_node(current_id) {
            Some(Some(n)) => n,
            _ => continue,
        };
        let current_g = g_score[&current_id];

        for (neighbor, edge_cost) in current.successors() {
            let tentative_g = current_g + edge_cost;
            let nid = neighbor.unique_id;
            if tentative_g < *g_score.get(&nid).unwrap_or(&usize::MAX) {
                came_from.insert(nid, current_id);
                g_score.insert(nid, tentative_g);
                let f = tentative_g + neighbor.distance(&goal_node);
                open.push((Reverse(f), nid));
            }
        }
    }

    Err(AstarError::NoPath)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register() {
        let json = std::fs::read_to_string("tests/rsc/ai_nodes_info.json").unwrap();
        assert!(register_nodes(&json).is_ok());
        assert!(NODES.with(|nodes_ref| !nodes_ref.borrow().is_empty()))
    }

    #[test]
    fn test_add_node() {
        let json = std::fs::read_to_string("tests/rsc/ai_nodes_info.json").unwrap();
        assert!(register_nodes(&json).is_ok());
        let mut node_to_add = NODES
            .with(|nodes_ref| nodes_ref.borrow().get(18).cloned())
            .unwrap()
            .unwrap()
            .as_ref()
            .clone();
        let initial_len = NODES.with(|nodes_ref| nodes_ref.borrow().len());

        node_to_add.unique_id = initial_len;
        assert!(add_node(&node_to_add.to_json_string()).is_ok());
        assert!(initial_len == NODES.with(|nodes_ref| nodes_ref.borrow().len() - 1));
    }

    #[test]
    fn test_remove_node() {
        let json = std::fs::read_to_string("tests/rsc/ai_nodes_info.json").unwrap();
        assert!(register_nodes(&json).is_ok());

        assert!(remove_node("11").is_ok());
        assert!(NODES.with(|nodes_ref| nodes_ref.borrow().get(11).unwrap().is_none()))
    }

    #[test]
    fn test_pathfinding() {
        let json = std::fs::read_to_string("tests/rsc/ai_nodes_info.json").unwrap();
        assert!(register_nodes(&json).is_ok());
        assert!(generate_path(10, 25).is_ok());
    }

    // --- Additional pathfinder tests ---

    #[test]
    fn test_node_from_json_valid() {
        let val = argus_json::parse_value(
            b"{\"unique_id\":0,\"x\":10,\"y\":20,\"z\":1,\"connected_nodes_id\":[1,2]}"
        ).unwrap();
        let node = Node::from_json(&val).unwrap();
        assert_eq!(node.unique_id, 0);
        assert_eq!(node.x, 10);
        assert_eq!(node.y, 20);
        assert_eq!(node.z, 1);
        assert_eq!(node.connected_nodes_id, vec![1, 2]);
    }

    #[test]
    fn test_node_from_json_missing_unique_id() {
        let val = argus_json::parse_value(
            b"{\"x\":10,\"y\":20,\"z\":1,\"connected_nodes_id\":[]}"
        ).unwrap();
        assert!(Node::from_json(&val).is_err());
    }

    #[test]
    fn test_node_from_json_missing_x() {
        let val = argus_json::parse_value(
            b"{\"unique_id\":0,\"y\":20,\"z\":1,\"connected_nodes_id\":[]}"
        ).unwrap();
        assert!(Node::from_json(&val).is_err());
    }

    #[test]
    fn test_node_from_json_missing_connected() {
        let val = argus_json::parse_value(
            b"{\"unique_id\":0,\"x\":10,\"y\":20,\"z\":1}"
        ).unwrap();
        assert!(Node::from_json(&val).is_err());
    }

    #[test]
    fn test_node_from_json_empty_connections() {
        let val = argus_json::parse_value(
            b"{\"unique_id\":0,\"x\":10,\"y\":20,\"z\":1,\"connected_nodes_id\":[]}"
        ).unwrap();
        let node = Node::from_json(&val).unwrap();
        assert!(node.connected_nodes_id.is_empty());
    }

    #[test]
    fn test_node_to_json_roundtrip() {
        let node = Node {
            unique_id: 5,
            x: 100,
            y: 200,
            z: 1,
            connected_nodes_id: vec![3, 7, 9],
        };
        let json_str = node.to_json_string();
        let val = argus_json::parse_value(json_str.as_bytes()).unwrap();
        let reconstructed = Node::from_json(&val).unwrap();
        assert_eq!(reconstructed.unique_id, 5);
        assert_eq!(reconstructed.x, 100);
        assert_eq!(reconstructed.y, 200);
        assert_eq!(reconstructed.z, 1);
        assert_eq!(reconstructed.connected_nodes_id, vec![3, 7, 9]);
    }

    #[test]
    fn test_node_distance() {
        let a = Node { unique_id: 0, x: 0, y: 0, z: 1, connected_nodes_id: vec![] };
        let b = Node { unique_id: 1, x: 3, y: 4, z: 1, connected_nodes_id: vec![] };
        assert_eq!(a.distance(&b), 5); // 3-4-5 triangle, integer sqrt
    }

    #[test]
    fn test_node_distance_same_point() {
        let a = Node { unique_id: 0, x: 5, y: 5, z: 1, connected_nodes_id: vec![] };
        assert_eq!(a.distance(&a), 0);
    }

    #[test]
    fn test_register_nodes_not_indexed() {
        // Nodes whose unique_ids don't match their positions
        let json = "[{\"unique_id\":1,\"x\":0,\"y\":0,\"z\":1,\"connected_nodes_id\":[]}]";
        let result = register_nodes(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_register_nodes_invalid_json() {
        assert!(register_nodes("not json").is_err());
    }

    #[test]
    fn test_register_nodes_not_array() {
        assert!(register_nodes("{\"not\":\"array\"}").is_err());
    }

    #[test]
    fn test_register_nodes_empty_graph() {
        let result = register_nodes("[]");
        assert!(result.is_ok());
    }

    #[test]
    fn test_remove_node_invalid_id() {
        assert!(remove_node("not_a_number").is_err());
    }

    #[test]
    fn test_remove_node_nonexistent() {
        // Attempt removing a node id that doesn't exist
        assert!(remove_node("999999").is_err());
    }
}
