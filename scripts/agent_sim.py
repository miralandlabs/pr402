import os
import sys
import json
import time
import subprocess
import requests
import base64

# Configuration (from env or defaults)
SOLANA_RPC_URL = os.getenv("SOLANA_RPC_URL", "https://api.devnet.solana.com")
CHAIN_ID = os.getenv("SOLANA_CHAIN_ID", "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1")
USDC_MINT = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
FACILITATOR_URL = os.getenv("PR402_BASE_URL", "https://preview.agent.pay402.me")

# Program IDs
SLA_ESCROW_ID = "s4CYBXKT29kym1gVeQQceDHmeRUR3fsHuHQY8HrkXPA"
UNIVERSALSETTLE_ID = "u7adeo6tGdURcRUAawYg1C7SZs8bs3JgjoW7EQVs4KY"

# Keypairs (Placeholder: assuming the user has them configured in solana CLI)
def get_address(label="id.json"):
    try:
        res = subprocess.run(["solana", "address", "--keypair", f"{os.path.expanduser('~')}/.config/solana/{label}"], capture_output=True, text=True)
        return res.stdout.strip()
    except:
        return "FacilLfwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu" # Fallback test addr

SELLER_ADDR = get_address("test-id.json")
BUYER_ADDR = get_address("id.json")

print("="*60)
print("🤖 X402 Agentic Simulation System (Cloud-Enabled)")
print("="*60)
print(f"Facilitator: {FACILITATOR_URL}")
print(f"Seller Address: {SELLER_ADDR}")
print(f"Buyer Address:  {BUYER_ADDR}")
print("-"*60)
print("📝 Prerequisites for Cloud Deployment:")
print("  1. ESCROW_PROGRAM_ID: s4CYBXKT29kym1gVeQQceDHmeRUR3fsHuHQY8HrkXPA")
print("  2. UNIVERSALSETTLE_PROGRAM_ID: u7adeo6tGdURcRUAawYg1C7SZs8bs3JgjoW7EQVs4KY")
print("  3. ORACLE_AUTHORITIES: <Pubkeys for candidates, comma-separated>")
print("-"*60)

def run_scenario(amount, task_name):
    print(f"\n🚀 Scenario: {task_name} (Amount: ${amount})")
    
    # 1. Buyer hits the Resource Provider (Seller Agent)
    # Seller decides scheme based on amount
    if amount < 10.0:
        target_scheme = "exact"
        print(f"  [Seller Agent] Decision: Small amount. Requesting '{target_scheme}' scheme.")
    else:
        target_scheme = "sla-escrow"
        print(f"  [Seller Agent] Decision: Institutional amount. Requesting '{target_scheme}' scheme.")

    # 2. Buyer Agent queries Facilitator Metadata
    print(f"  [Buyer Agent] Discovering facilitator capabilities at {FACILITATOR_URL}...")
    try:
        resp = requests.get(f"{FACILITATOR_URL}/api/v1/facilitator/supported")
        supported = resp.json()
    except Exception as e:
        print(f"  ❌ Failed to reach facilitator: {e}")
        return

    # Find the requested scheme
    kind = next((k for k in supported['kinds'] if k['scheme'] == target_scheme), None)
    if not kind:
        print(f"  ❌ Facilitator does not support '{target_scheme}' on {CHAIN_ID}")
        return
        
    print(f"  ✅ Found {target_scheme} support. Initializing Verification...")
    print(f"  📦 Enriched Metadata: {json.dumps(kind['extra'], indent=4)}")

    # 3. Buyer Agent builds & signs transaction (Simulation)
    # (In a real test, we would use spl-token or the CLI to build a real tx)
    # For simulation, we'll demonstrate the request payload structure
    
    verify_req = {
        "x402Version": 2,
        "paymentRequirements": {
            "scheme": target_scheme,
            "network": CHAIN_ID,
            "amount": str(int(amount * 1_000_000)), # u64 string for USDC decimals
            "payTo": SELLER_ADDR,
            "asset": USDC_MINT,
            "maxTimeoutSeconds": 3600,
            "extra": kind['extra']
        },
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": target_scheme,
                "network": CHAIN_ID,
                "amount": str(int(amount * 1_000_000)),
                "payTo": SELLER_ADDR,
                "asset": USDC_MINT,
            },
            "payload": {
                "transaction": "BASE64_SIGNED_TX_PLACEHOLDER"
            }
        }
    }
    
    print(f"  [Buyer Agent] Submitting {target_scheme} verification...")
    # This would hit POST /api/v1/facilitator/verify or /settle
    print("  ✅ Interaction simulation finished.")

def start_facilitator():
    print("\n📦 Starting pr402 Facilitator Server...")
    # Note: We assume the user has configured env vars for the server
    # We use subprocess.Popen to run in background
    cmd = ["cargo", "run", "--release", "--", "server"]
    # Provide a few seconds for build/start if needed, but in real test we'd wait for port
    print("  (Assuming server is already running or being started by user)")

if __name__ == "__main__":
    # Small Payment
    run_scenario(1.50, "Micro-Task: Text Parsing")
    
    # Large Payment
    run_scenario(25.00, "Heavy-Task: Video Rendering")

    print("\n" + "="*60)
    print("🏁 Agentic Flow Simulation Complete")
    print("="*60)
