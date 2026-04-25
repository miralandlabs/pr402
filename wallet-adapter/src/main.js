import React, { useMemo, useCallback, useEffect } from "react";
import ReactDOM from "react-dom/client";
import {
    ConnectionProvider,
    WalletProvider,
    useWallet,
} from "@solana/wallet-adapter-react";
import {
    BaseWalletMultiButton,
    WalletModalProvider,
} from "@solana/wallet-adapter-react-ui";
import { PhantomWalletAdapter } from "@solana/wallet-adapter-phantom";
import { SolflareWalletAdapter } from "@solana/wallet-adapter-solflare";
import { TrustWalletAdapter } from "@solana/wallet-adapter-trust";
import { BackpackWalletAdapter } from "@solana/wallet-adapter-backpack";
import { Connection, VersionedTransaction } from "@solana/web3.js";
import * as buffer from "buffer";

window.Buffer = buffer.Buffer;

require("./styles.css");

const LABELS = {
    "change-wallet": "Change wallet",
    connecting: "Connecting ...",
    "copy-address": "Copy address",
    copied: "Copied",
    disconnect: "Disconnect",
    "has-wallet": "Connect",
    "no-wallet": "Connect",
    phantom: "Phantom",
    solflare: "Solflare",
    trust: "Trust",
    backpack: "Backpack",
};

function pr402RpcEndpoint() {
    if (typeof window !== "undefined" && window.__PR402_RPC_ENDPOINT__) {
        return window.__PR402_RPC_ENDPOINT__;
    }
    return "https://api.mainnet-beta.solana.com";
}

export const Wallet = () => {
    const endpoint = pr402RpcEndpoint();
    const wallets = useMemo(
        () => [
            new PhantomWalletAdapter(),
            new SolflareWalletAdapter(),
            new TrustWalletAdapter(),
            new BackpackWalletAdapter(),
        ],
        []
    );
    return (
        <ConnectionProvider endpoint={endpoint}>
            <WalletProvider wallets={wallets} autoConnect={true}>
                <WalletModalProvider>
                    {/*
                      Hidden trigger: real control is the landing-page "Connect wallet" button
                      that calls ShowWalletModal(). Kept focusable-sized off-screen so .click() works.
                    */}
                    <div
                        className="pr402-wallet-adapter-hidden-trigger"
                        aria-hidden="true"
                    >
                        <BaseWalletMultiButton labels={LABELS} />
                    </div>
                    <Dispatcher />
                    <Disconnect />
                    <SignMessage />
                    <SignTransaction />
                </WalletModalProvider>
            </WalletProvider>
        </ConnectionProvider>
    );
};

function MountWalletAdapter() {
    const container = document.getElementById("miracle-wallet-adapter");
    if (!container) {
        console.error("pr402 wallet: missing #miracle-wallet-adapter root");
        return;
    }
    const root = ReactDOM.createRoot(container);
    root.render(<Wallet />);

    setTimeout(() => {
        const walletButtons = document.querySelectorAll(
            ".wallet-adapter-modal-list .wallet-adapter-button"
        );
        walletButtons.forEach((button) => {
            const textElement = button.querySelector("span");
            if (textElement && textElement.textContent.includes("Detected")) {
                const originalText = textElement.textContent;
                const spacedText = originalText.replace(
                    "Detected",
                    " Detected"
                );
                textElement.setAttribute("data-wallet-name", spacedText);
            }
        });
    }, 100);
}

function pr402WalletMountRoot() {
    return document.getElementById("miracle-wallet-adapter");
}

/**
 * Only click the trigger inside our React mount. A global querySelector matches
 * extension-injected or third-party UI (e.g. opening a wallet vendor homepage).
 */
function ShowWalletModal() {
    const root = pr402WalletMountRoot();
    if (!root) return;
    const walletButton = root.querySelector(".wallet-adapter-button-trigger");
    if (walletButton) {
        walletButton.click();
    }
}

function DisconnectWallet() {
    const root = pr402WalletMountRoot();
    if (root) {
        const disconnectButton = root.querySelector(
            ".wallet-adapter-button-trigger"
        );
        if (disconnectButton) {
            disconnectButton.click();
        }
    }

    const keysToRemove = [
        "walletName",
        "walletAdapter",
        "wallet_address",
        "solana_rpc_url",
        "wallet-adapter-auto-connect-enabled",
    ];

    keysToRemove.forEach((key) => {
        localStorage.removeItem(key);
    });

    if (window.solana && window.solana.disconnect) {
        window.solana.disconnect();
    }
    if (window.solflare && window.solflare.disconnect) {
        window.solflare.disconnect();
    }
    if (window.backpack && window.backpack.disconnect) {
        window.backpack.disconnect();
    }

    const container = document.getElementById("miracle-wallet-adapter");
    if (container) {
        container.innerHTML = "";
        setTimeout(() => {
            window.MountWalletAdapter();
        }, 100);
    }
}

function Disconnect() {
    const { publicKey, disconnect } = useWallet();
    const callback = useCallback(
        async (_) => {
            try {
                await disconnect();
            } catch (err) {
                console.log(err);
            }
        },
        [publicKey, disconnect]
    );
    window.MiracleWalletDisconnecter = callback;
    return null;
}

window.MountWalletAdapter = MountWalletAdapter;
window.ShowWalletModal = ShowWalletModal;
window.MiracleWalletDisconnecter = DisconnectWallet;

window.ClearWalletStorage = function () {
    const keysToRemove = [
        "walletName",
        "walletAdapter",
        "wallet_address",
        "solana_rpc_url",
        "wallet-adapter-auto-connect-enabled",
    ];

    keysToRemove.forEach((key) => {
        localStorage.removeItem(key);
    });

    const container = document.getElementById("miracle-wallet-adapter");
    if (container) {
        container.innerHTML = "";
        setTimeout(() => {
            window.MountWalletAdapter();
        }, 100);
    }
};

/**
 * Sign a facilitator-built VersionedTransaction (base64) and submit it on the
 * same RPC as ConnectionProvider (window.__PR402_RPC_ENDPOINT__).
 *
 * @param {string} unsignedTxB64 - Unsigned tx bytes, base64
 * @returns {Promise<string>} Transaction signature (base58)
 */
window.MiracleSignAndSendVersionedTxB64 = async function (unsignedTxB64) {
    const rpc = pr402RpcEndpoint();
    const connection = new Connection(rpc, "confirmed");
    if (!window.MiracleTxSigner) {
        throw new Error("Wallet adapter not ready (MiracleTxSigner)");
    }
    const signedB64 = await window.MiracleTxSigner({ b64: unsignedTxB64 });
    if (!signedB64) {
        throw new Error("Wallet signing returned empty result");
    }
    const signedTx = VersionedTransaction.deserialize(
        Buffer.from(signedB64, "base64")
    );
    const sig = await connection.sendRawTransaction(signedTx.serialize(), {
        skipPreflight: false,
        maxRetries: 5,
    });
    const confirmation = await connection.confirmTransaction(sig, "confirmed");
    if (confirmation.value.err) {
        throw new Error(JSON.stringify(confirmation.value.err));
    }
    return sig;
};

function Dispatcher() {
    const { publicKey } = useWallet();
    useEffect(() => {
        const pubkeyBase58 = publicKey ? publicKey.toBase58() : null;
        window.__PR402_CONNECTED_PUBKEY__ = pubkeyBase58;
        try {
            const event = new CustomEvent("miracle-pubkey", {
                bubbles: true,
                detail: {
                    pubkey: publicKey
                        ? Array.from(publicKey.toBytes())
                        : null,
                    pubkeyBase58,
                },
            });
            window.dispatchEvent(event);
        } catch (err) {
            console.log(err);
        }
    }, [publicKey]);
    return null;
}

function SignTransaction() {
    const { publicKey, signTransaction } = useWallet();
    const callback = useCallback(
        async (msg) => {
            try {
                const tx = VersionedTransaction.deserialize(
                    Buffer.from(msg.b64, "base64")
                );
                const signed = await signTransaction(tx);
                const b64 = Buffer.from(signed.serialize()).toString("base64");
                return b64;
            } catch (err) {
                console.log(err);
            }
        },
        [publicKey, signTransaction]
    );
    window.MiracleTxSigner = callback;
    return null;
}

function SignMessage() {
    const { publicKey, signMessage } = useWallet();
    const callback = useCallback(
        async (msg_obj) => {
            try {
                let messageBytes;
                if (typeof msg_obj === "string") {
                    messageBytes = new TextEncoder().encode(msg_obj);
                } else if (msg_obj && msg_obj.b64) {
                    const binaryString = atob(msg_obj.b64);
                    messageBytes = new Uint8Array(binaryString.length);
                    for (let i = 0; i < binaryString.length; i++) {
                        messageBytes[i] = binaryString.charCodeAt(i);
                    }
                } else {
                    throw new Error("Invalid message format");
                }

                const signedMessage = await signMessage(messageBytes);

                const signatureBase64 = btoa(
                    String.fromCharCode(...signedMessage)
                );
                return signatureBase64;
            } catch (err) {
                console.log(err);
                throw err;
            }
        },
        [publicKey, signMessage]
    );
    window.MiracleMessageSigner = callback;
    return null;
}
