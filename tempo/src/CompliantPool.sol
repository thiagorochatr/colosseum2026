// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP403Registry} from "tempo-std/interfaces/ITIP403Registry.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";
import {PoseidonT3} from "poseidon-solidity/PoseidonT3.sol";

interface IWithdrawVerifier {
    function verifyProof(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256[4] calldata publicSignals
    ) external view returns (bool);
}

/// A private pool for one TIP-20 stablecoin at one fixed denomination, whose
/// clean set is derived from the token issuer's own TIP-403 policy.
///
/// A withdrawal proves that its note sits in the pool tree and in the clean
/// tree, without saying which note. The clean tree is the pool tree with every
/// deposit whose depositor is no longer an authorized sender set to zero.
/// Anyone can apply that rule, through exclude and reinclude, and the contract
/// checks it against the registry. There is no curator. See docs/SPEC.md §7.
contract CompliantPool {
    uint256 public constant FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    uint256 public constant DEPTH = 20;
    uint256 public constant ROOT_HISTORY = 32;
    string public constant BINDING_DOMAIN = "compliance-pool:tempo:withdraw:v1";

    ITIP403Registry internal constant REGISTRY = StdPrecompiles.TIP403_REGISTRY;

    ITIP20 public immutable token;
    uint256 public immutable denomination;
    IWithdrawVerifier public immutable verifier;

    /// Number of deposits so far, and the index of the next one.
    uint256 public nextIndex;

    /// Pool tree: an append-only frontier and a ring of recent roots.
    uint256[DEPTH] internal filled;
    uint256[ROOT_HISTORY] public poolRoots;
    uint256 public poolRootPos;

    /// Clean tree: every node stored, so any leaf can be rewritten. An unset
    /// node is the empty subtree at its level. Key: level << 32 | index.
    mapping(uint256 => uint256) internal cleanNodes;
    /// Clean roots, each tagged with the epoch it was pushed in. An exclusion
    /// starts a new epoch, and only roots of the current epoch are accepted.
    uint256[ROOT_HISTORY] public cleanRoots;
    uint64[ROOT_HISTORY] public cleanRootEpochs;
    uint256 public cleanRootPos;
    uint64 public cleanEpoch;

    mapping(uint256 => address) public depositor;
    mapping(uint256 => uint256) public commitmentAt;
    mapping(uint256 => bool) public spent;

    event Deposit(uint256 indexed index, uint256 commitment, address indexed depositor, uint256 poolRoot, uint256 cleanRoot);
    event Excluded(uint256 indexed index, address indexed depositor, uint256 cleanRoot, uint64 cleanEpoch);
    event Reincluded(uint256 indexed index, uint256 cleanRoot);
    event Withdrawal(uint256 indexed nullifierHash, address indexed recipient, address indexed relayer, uint256 fee);

    error InvalidCommitment();
    error PoolFull();
    error TransferFailed();
    error UnknownIndex();
    error AlreadyExcluded();
    error NotExcluded();
    error StillAuthorized();
    error StillUnauthorized();
    error InvalidRecipient();
    error FeeTooHigh();
    error RelayerRequired();
    error NullifierSpent();
    error UnknownPoolRoot();
    error StaleCleanRoot();
    error InvalidProof();

    constructor(ITIP20 token_, uint256 denomination_, IWithdrawVerifier verifier_) {
        require(address(token_) != address(0) && address(verifier_) != address(0) && denomination_ > 0, "config");
        token = token_;
        denomination = denomination_;
        verifier = verifier_;
    }

    // ------------------------------------------------------------------ deposit

    /// Escrows one denomination from the caller and inserts `commitment` in both
    /// trees. The token's own policy already refuses an unauthorized sender.
    function deposit(uint256 commitment) external returns (uint256 index) {
        if (commitment == 0 || commitment >= FIELD) revert InvalidCommitment();
        index = nextIndex;
        if (index >= 2 ** DEPTH) revert PoolFull();

        // Checks, effects, then the one interaction. A failed transfer reverts
        // everything, so the order only matters against a token that calls back.
        uint256 poolRoot = _insertPool(index, commitment);
        uint256 cleanRoot = _writeClean(index, commitment);
        depositor[index] = msg.sender;
        commitmentAt[index] = commitment;
        nextIndex = index + 1;
        _pushPoolRoot(poolRoot);
        _pushCleanRoot(cleanRoot);
        emit Deposit(index, commitment, msg.sender, poolRoot, cleanRoot);

        if (!token.transferFrom(msg.sender, address(this), denomination)) revert TransferFailed();
    }

    // ---------------------------------------------------------------- the rule

    /// Removes deposit `index` from the clean set because its depositor is no
    /// longer an authorized sender under the token's current policy. Callable by
    /// anyone; the registry decides.
    function exclude(uint256 index) external {
        if (index >= nextIndex) revert UnknownIndex();
        if (cleanNodes[_key(0, index)] == 0) revert AlreadyExcluded();
        address who = depositor[index];
        if (REGISTRY.isAuthorizedSender(token.transferPolicyId(), who)) revert StillAuthorized();
        uint256 root = _writeClean(index, 0);
        uint64 epoch = cleanEpoch + 1;
        cleanEpoch = epoch;
        _pushCleanRoot(root);
        emit Excluded(index, who, root, epoch);
    }

    /// Restores deposit `index` once its depositor is authorized again.
    /// Adding a leaf cannot help an excluded note, so the epoch does not move.
    function reinclude(uint256 index) external {
        if (index >= nextIndex) revert UnknownIndex();
        if (cleanNodes[_key(0, index)] != 0) revert NotExcluded();
        if (!REGISTRY.isAuthorizedSender(token.transferPolicyId(), depositor[index])) revert StillUnauthorized();
        uint256 root = _writeClean(index, commitmentAt[index]);
        _pushCleanRoot(root);
        emit Reincluded(index, root);
    }

    // ----------------------------------------------------------------- withdraw

    /// Pays one denomination, minus `fee`, to `recipient` against a proof that
    /// some unspent clean note exists. Anyone may submit it; `relayer` gets `fee`.
    function withdraw(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256 poolRoot,
        uint256 cleanRoot,
        uint256 nullifierHash,
        address recipient,
        address relayer,
        uint256 fee
    ) external {
        if (recipient == address(0)) revert InvalidRecipient();
        if (fee > denomination) revert FeeTooHigh();
        if (fee > 0 && relayer == address(0)) revert RelayerRequired();
        if (spent[nullifierHash]) revert NullifierSpent();
        if (!isKnownPoolRoot(poolRoot)) revert UnknownPoolRoot();
        if (!isValidCleanRoot(cleanRoot)) revert StaleCleanRoot();

        uint256[4] memory signals = [poolRoot, cleanRoot, nullifierHash, bindingFor(recipient, relayer, fee)];
        if (!verifier.verifyProof(a, b, c, signals)) revert InvalidProof();

        spent[nullifierHash] = true;
        emit Withdrawal(nullifierHash, recipient, relayer, fee);
        if (!token.transfer(recipient, denomination - fee)) revert TransferFailed();
        if (fee > 0 && !token.transfer(relayer, fee)) revert TransferFailed();
    }

    // -------------------------------------------------------------------- views

    /// The action binding of docs/SPEC.md §6: keccak with its first byte cleared.
    function bindingFor(address recipient, address relayer, uint256 fee) public view returns (uint256) {
        bytes32 d = keccak256(abi.encodePacked(BINDING_DOMAIN, block.chainid, address(this), recipient, relayer, fee));
        return uint256(d) & (type(uint256).max >> 8);
    }

    function isKnownPoolRoot(uint256 root) public view returns (bool) {
        if (root == 0) return false;
        for (uint256 i = 0; i < ROOT_HISTORY; i++) {
            if (poolRoots[i] == root) return true;
        }
        return false;
    }

    function isValidCleanRoot(uint256 root) public view returns (bool) {
        if (root == 0) return false;
        uint64 epoch = cleanEpoch;
        for (uint256 i = 0; i < ROOT_HISTORY; i++) {
            if (cleanRoots[i] == root && cleanRootEpochs[i] == epoch) return true;
        }
        return false;
    }

    function currentPoolRoot() external view returns (uint256) {
        return poolRoots[poolRootPos];
    }

    function currentCleanRoot() external view returns (uint256) {
        return cleanRoots[cleanRootPos];
    }

    /// The clean tree's leaf at `index`: the commitment, or 0 if excluded.
    function cleanLeaf(uint256 index) external view returns (uint256) {
        return cleanNodes[_key(0, index)];
    }

    // ---------------------------------------------------------------- internals

    function _key(uint256 level, uint256 index) internal pure returns (uint256) {
        return (level << 32) | index;
    }

    function _cleanNode(uint256 level, uint256 index) internal view returns (uint256) {
        uint256 v = cleanNodes[_key(level, index)];
        return v == 0 ? zero(level) : v;
    }

    /// Appends `leaf` at `index` to the pool frontier and returns the new root.
    function _insertPool(uint256 index, uint256 leaf) internal returns (uint256 node) {
        node = leaf;
        uint256 i = index;
        for (uint256 level = 0; level < DEPTH; level++) {
            if (i & 1 == 0) {
                filled[level] = node;
                node = PoseidonT3.hash([node, zero(level)]);
            } else {
                node = PoseidonT3.hash([filled[level], node]);
            }
            i >>= 1;
        }
    }

    /// Writes `leaf` at `index` in the clean tree, refreshes its path, and returns the new root.
    function _writeClean(uint256 index, uint256 leaf) internal returns (uint256 node) {
        node = leaf;
        uint256 i = index;
        cleanNodes[_key(0, i)] = leaf;
        for (uint256 level = 0; level < DEPTH; level++) {
            uint256 sibling = _cleanNode(level, i ^ 1);
            node = i & 1 == 0 ? PoseidonT3.hash([node, sibling]) : PoseidonT3.hash([sibling, node]);
            i >>= 1;
            if (level + 1 < DEPTH) cleanNodes[_key(level + 1, i)] = node;
        }
    }

    function _pushPoolRoot(uint256 root) internal {
        uint256 pos = (poolRootPos + 1) % ROOT_HISTORY;
        poolRoots[pos] = root;
        poolRootPos = pos;
    }

    function _pushCleanRoot(uint256 root) internal {
        uint256 pos = (cleanRootPos + 1) % ROOT_HISTORY;
        cleanRoots[pos] = root;
        cleanRootEpochs[pos] = cleanEpoch;
        cleanRootPos = pos;
    }

    /// Root of an empty subtree of height `level`. mirror-core ZERO_LADDER.
    function zero(uint256 level) internal pure returns (uint256) {
        if (level == 0) return 0x0000000000000000000000000000000000000000000000000000000000000000;
        else if (level == 1) return 0x2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864;
        else if (level == 2) return 0x1069673dcdb12263df301a6ff584a7ec261a44cb9dc68df067a4774460b1f1e1;
        else if (level == 3) return 0x18f43331537ee2af2e3d758d50f72106467c6eea50371dd528d57eb2b856d238;
        else if (level == 4) return 0x07f9d837cb17b0d36320ffe93ba52345f1b728571a568265caac97559dbc952a;
        else if (level == 5) return 0x2b94cf5e8746b3f5c9631f4c5df32907a699c58c94b2ad4d7b5cec1639183f55;
        else if (level == 6) return 0x2dee93c5a666459646ea7d22cca9e1bcfed71e6951b953611d11dda32ea09d78;
        else if (level == 7) return 0x078295e5a22b84e982cf601eb639597b8b0515a88cb5ac7fa8a4aabe3c87349d;
        else if (level == 8) return 0x2fa5e5f18f6027a6501bec864564472a616b2e274a41211a444cbe3a99f3cc61;
        else if (level == 9) return 0x0e884376d0d8fd21ecb780389e941f66e45e7acce3e228ab3e2156a614fcd747;
        else if (level == 10) return 0x1b7201da72494f1e28717ad1a52eb469f95892f957713533de6175e5da190af2;
        else if (level == 11) return 0x1f8d8822725e36385200c0b201249819a6e6e1e4650808b5bebc6bface7d7636;
        else if (level == 12) return 0x2c5d82f66c914bafb9701589ba8cfcfb6162b0a12acf88a8d0879a0471b5f85a;
        else if (level == 13) return 0x14c54148a0940bb820957f5adf3fa1134ef5c4aaa113f4646458f270e0bfbfd0;
        else if (level == 14) return 0x190d33b12f986f961e10c0ee44d8b9af11be25588cad89d416118e4bf4ebe80c;
        else if (level == 15) return 0x22f98aa9ce704152ac17354914ad73ed1167ae6596af510aa5b3649325e06c92;
        else if (level == 16) return 0x2a7c7c9b6ce5880b9f6f228d72bf6a575a526f29c66ecceef8b753d38bba7323;
        else if (level == 17) return 0x2e8186e558698ec1c67af9c14d463ffc470043c9c2988b954d75dd643f36b992;
        else if (level == 18) return 0x0f57c5571e9a4eab49e2c8cf050dae948aef6ead647392273546249d1c1ff10f;
        else if (level == 19) return 0x1830ee67b5fb554ad5f63d4388800e1cfe78e310697d46e43c9ce36134f72cca;
        else if (level == 20) return 0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3e;
        revert("level");
    }}
