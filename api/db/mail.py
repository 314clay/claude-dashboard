"""Mail network data for Gas Town agent communication graph.

Reads mail messages from town-level beads and builds a network graph
showing who-talks-to-who between agents.
"""
import json
from pathlib import Path
from collections import defaultdict
from typing import List, Dict, Any, Optional
from dataclasses import dataclass


@dataclass
class MailNode:
    """Node in the mail network (an agent)."""
    id: str
    label: str
    message_count: int  # Total messages sent + received
    sent_count: int
    received_count: int


@dataclass
class MailEdge:
    """Edge in the mail network (messages between agents)."""
    source: str  # Sender agent ID
    target: str  # Recipient agent ID
    weight: float  # Normalized 0-1 based on message count
    message_count: int  # Raw count


def load_mail_beads() -> List[Dict[str, Any]]:
    """Load mail beads from town-level issues.jsonl."""
    beads_path = Path.home() / "gt" / ".beads" / "issues.jsonl"

    if not beads_path.exists():
        return []

    mail_beads = []
    with open(beads_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                bead = json.loads(line)
                # Filter to only message type beads
                if bead.get("issue_type") == "message":
                    mail_beads.append(bead)
            except json.JSONDecodeError:
                continue

    return mail_beads


def normalize_agent_id(agent_id: Optional[str]) -> str:
    """Normalize agent ID for consistent grouping.

    Handles variations like:
    - "lingoswipe/topaz" -> "lingoswipe/topaz"
    - "mayor/" -> "mayor"
    - None -> "unknown"
    """
    if not agent_id:
        return "unknown"

    # Strip trailing slash
    agent_id = agent_id.rstrip("/")

    # Clean up any whitespace
    agent_id = agent_id.strip()

    return agent_id or "unknown"


def get_sender_from_bead(bead: Dict[str, Any]) -> str:
    """Extract sender from bead, checking labels for explicit from: tag."""
    # Check for explicit from: label first
    labels = bead.get("labels", [])
    for label in labels:
        if isinstance(label, str) and label.startswith("from:"):
            return normalize_agent_id(label[5:])

    # Fall back to created_by
    return normalize_agent_id(bead.get("created_by"))


def get_recipient_from_bead(bead: Dict[str, Any]) -> str:
    """Extract recipient from bead assignee field."""
    return normalize_agent_id(bead.get("assignee"))


def get_mail_network() -> Dict[str, Any]:
    """Build mail network graph data.

    Returns:
        {
            "nodes": [{"id": "agent", "label": "Agent", "message_count": N, ...}],
            "edges": [{"source": "a", "target": "b", "weight": 0.5, "message_count": N}],
            "stats": {"total_messages": N, "agent_count": M}
        }
    """
    beads = load_mail_beads()

    if not beads:
        return {"nodes": [], "edges": [], "stats": {"total_messages": 0, "agent_count": 0}}

    # Count messages between agents
    edge_counts: Dict[tuple, int] = defaultdict(int)
    agent_sent: Dict[str, int] = defaultdict(int)
    agent_received: Dict[str, int] = defaultdict(int)

    for bead in beads:
        sender = get_sender_from_bead(bead)
        recipient = get_recipient_from_bead(bead)

        # Skip self-messages
        if sender == recipient:
            continue

        # Count this edge (directed: sender -> recipient)
        edge_counts[(sender, recipient)] += 1
        agent_sent[sender] += 1
        agent_received[recipient] += 1

    # Get all unique agents
    all_agents = set(agent_sent.keys()) | set(agent_received.keys())

    # Find max edge weight for normalization
    max_count = max(edge_counts.values()) if edge_counts else 1

    # Build nodes
    nodes = []
    for agent in sorted(all_agents):
        sent = agent_sent.get(agent, 0)
        received = agent_received.get(agent, 0)
        nodes.append({
            "id": agent,
            "label": agent.split("/")[-1] if "/" in agent else agent,
            "full_label": agent,
            "message_count": sent + received,
            "sent_count": sent,
            "received_count": received,
        })

    # Build edges
    edges = []
    for (sender, recipient), count in edge_counts.items():
        edges.append({
            "source": sender,
            "target": recipient,
            "weight": count / max_count,  # Normalize to 0-1
            "message_count": count,
        })

    return {
        "nodes": nodes,
        "edges": edges,
        "stats": {
            "total_messages": len(beads),
            "agent_count": len(all_agents),
            "max_edge_count": max_count,
        }
    }
