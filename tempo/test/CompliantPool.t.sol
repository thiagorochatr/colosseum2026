// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP20RolesAuth} from "tempo-std/interfaces/ITIP20RolesAuth.sol";
import {ITIP403Registry} from "tempo-std/interfaces/ITIP403Registry.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";
import {PoseidonT4} from "poseidon-solidity/PoseidonT4.sol";
import {CompliantPool, IWithdrawVerifier} from "../src/CompliantPool.sol";
import {Groth16Verifier} from "../src/Groth16Verifier.sol";

/// The pool end to end against Tempo's precompiles, with real proofs from the
/// ceremony's final key. Run with `forge test --network tempo`.
///
/// Every proof is made off-chain by the SDK for the exact state the test built,
/// so each test also checks that the Solidity trees and the SDK's trees agree.
contract CompliantPoolTest is Test {
    ITIP403Registry constant REGISTRY = StdPrecompiles.TIP403_REGISTRY;
    ITIP20 constant PATH_USD = ITIP20(0x20C0000000000000000000000000000000000000);
    uint256 constant FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    uint256 constant DENOMINATION = 100e6;

    ITIP20 token;
    CompliantPool pool;
    uint64 policy;

    address[] users;
    address relayer = makeAddr("relayer");
    uint256[] leaves;
    uint256[] excluded;

    struct Proof {
        bool ok;
        uint256[2] a;
        uint256[2][2] b;
        uint256[2] c;
        uint256 poolRoot;
        uint256 cleanRoot;
        uint256 nullifierHash;
    }

    function setUp() public {
        token = ITIP20(
            StdPrecompiles.TIP20_FACTORY
            .createToken("Demo USD", "dUSD", "USD", PATH_USD, address(this), bytes32("pool"))
        );
        ITIP20RolesAuth(address(token)).grantRole(token.ISSUER_ROLE(), address(this));
        policy = REGISTRY.createPolicy(address(this), ITIP403Registry.PolicyType.BLACKLIST);
        token.changeTransferPolicyId(policy);

        pool = new CompliantPool(token, DENOMINATION, IWithdrawVerifier(address(new Groth16Verifier())));
        for (uint256 i = 0; i < 4; i++) {
            address u = makeAddr(string.concat("user", vm.toString(i)));
            users.push(u);
            token.mint(u, 1_000e6);
            vm.prank(u);
            token.approve(address(pool), type(uint256).max);
        }
    }

    // ------------------------------------------------------------------ helpers

    function noteK(uint256 i) internal pure returns (uint256) {
        return uint256(keccak256(abi.encode("k", i))) % FIELD;
    }

    function noteR(uint256 i) internal pure returns (uint256) {
        return uint256(keccak256(abi.encode("r", i))) % FIELD;
    }

    function commitment(uint256 i) internal pure returns (uint256) {
        return PoseidonT4.hash([noteK(i), noteR(i), DENOMINATION]);
    }

    /// User `u` deposits note `i`.
    function depositAs(uint256 u, uint256 i) internal returns (uint256 index) {
        // Computed first: the Poseidon library is an external call, and vm.prank
        // applies to the next external call.
        uint256 c = commitment(i);
        vm.prank(users[u]);
        index = pool.deposit(c);
        leaves.push(c);
    }

    function csv(uint256[] memory xs) internal pure returns (string memory s) {
        if (xs.length == 0) return "-";
        s = vm.toString(xs[0]);
        for (uint256 j = 1; j < xs.length; j++) {
            s = string.concat(s, ",", vm.toString(xs[j]));
        }
    }

    /// Proves note `i` at `index` against the trees as the SDK sees them, with
    /// `excludedView` as the clean mask, for the given action.
    function proveWith(uint256 i, uint256 index, uint256[] memory excludedView, address recipient, uint256 fee)
        internal
        returns (Proof memory p)
    {
        string[] memory cmd = new string[](9);
        cmd[0] = "node";
        cmd[1] = "script/ffi-prove.mjs";
        cmd[2] = vm.toString(noteK(i));
        cmd[3] = vm.toString(noteR(i));
        cmd[4] = vm.toString(DENOMINATION);
        cmd[5] = vm.toString(index);
        cmd[6] = vm.toString(pool.bindingFor(recipient, relayer, fee));
        cmd[7] = csv(excludedView);
        cmd[8] = csv(leaves);
        (p.ok, p.a, p.b, p.c, p.poolRoot, p.cleanRoot, p.nullifierHash) =
            abi.decode(vm.ffi(cmd), (bool, uint256[2], uint256[2][2], uint256[2], uint256, uint256, uint256));
    }

    function prove(uint256 i, uint256 index, address recipient, uint256 fee) internal returns (Proof memory) {
        return proveWith(i, index, excluded, recipient, fee);
    }

    function withdraw(Proof memory p, address recipient, uint256 fee) internal {
        withdraw(p, recipient, relayer, fee);
    }

    function withdraw(Proof memory p, address recipient, address by, uint256 fee) internal {
        pool.withdraw(p.a, p.b, p.c, p.poolRoot, p.cleanRoot, p.nullifierHash, recipient, by, fee);
    }

    function blockAndExclude(uint256 u, uint256 index) internal {
        REGISTRY.modifyPolicyBlacklist(policy, users[u], true);
        pool.exclude(index);
        excluded.push(index);
    }

    // -------------------------------------------------------------------- tests

    function test_deposit_escrows_and_both_trees_agree_with_the_sdk() public {
        uint256 before = token.balanceOf(users[0]);
        uint256 g = gasleft();
        depositAs(0, 0);
        console.log("deposit, first, gas:", g - gasleft());
        g = gasleft();
        depositAs(1, 1);
        console.log("deposit, second, gas:", g - gasleft());
        depositAs(2, 2);

        assertEq(token.balanceOf(users[0]), before - DENOMINATION);
        assertEq(token.balanceOf(address(pool)), 3 * DENOMINATION);
        Proof memory p = prove(1, 1, users[3], 0);
        assertTrue(p.ok);
        assertEq(p.poolRoot, pool.currentPoolRoot(), "Solidity and SDK pool roots");
        assertEq(p.cleanRoot, pool.currentCleanRoot(), "Solidity and SDK clean roots");
        assertEq(p.poolRoot, p.cleanRoot, "nothing excluded: the mask is the pool");
    }

    function test_a_clean_note_withdraws_to_a_fresh_address_and_pays_the_relayer() public {
        depositAs(0, 0);
        depositAs(1, 1);
        depositAs(2, 2);
        address fresh = makeAddr("fresh");
        uint256 fee = 1e6;
        Proof memory p = prove(1, 1, fresh, fee);
        uint256 g = gasleft();
        withdraw(p, fresh, fee);
        console.log("withdraw, gas:", g - gasleft());
        assertEq(token.balanceOf(fresh), DENOMINATION - fee);
        assertEq(token.balanceOf(relayer), fee);
        assertTrue(pool.spent(p.nullifierHash));
    }

    function test_a_note_cannot_be_spent_twice() public {
        depositAs(0, 0);
        depositAs(1, 1);
        Proof memory p = prove(0, 0, users[3], 0);
        withdraw(p, users[3], 0);
        vm.expectRevert(CompliantPool.NullifierSpent.selector);
        withdraw(p, users[3], 0);
    }

    function test_a_relayer_cannot_redirect_the_payment_or_raise_the_fee() public {
        depositAs(0, 0);
        depositAs(1, 1);
        Proof memory p = prove(0, 0, users[3], 1e6);
        vm.expectRevert(CompliantPool.InvalidProof.selector);
        withdraw(p, relayer, 1e6);
        vm.expectRevert(CompliantPool.InvalidProof.selector);
        withdraw(p, users[3], 2e6);
        withdraw(p, users[3], 1e6);
    }

    function test_a_relayer_cannot_take_the_fee_bound_to_another_relayer() public {
        depositAs(0, 0);
        depositAs(1, 1);
        Proof memory p = prove(0, 0, users[3], 1e6);
        address other = makeAddr("other relayer");
        vm.expectRevert(CompliantPool.InvalidProof.selector);
        withdraw(p, users[3], other, 1e6);
        withdraw(p, users[3], 1e6);
        assertEq(token.balanceOf(relayer), 1e6);
    }

    function test_blocking_a_depositor_after_the_fact_removes_their_note() public {
        depositAs(0, 0);
        depositAs(1, 1);
        depositAs(2, 2);
        // An honest proof for note 1 made before the block, still unsubmitted.
        Proof memory early = prove(1, 1, users[3], 0);

        uint64 epochBefore = pool.cleanEpoch();
        uint256 g = gasleft();
        blockAndExclude(1, 1);
        console.log("block and exclude, gas:", g - gasleft());
        assertEq(pool.cleanEpoch(), epochBefore + 1);
        assertEq(pool.cleanLeaf(1), 0);

        // The proof made against the old clean root is now stale.
        vm.expectRevert(CompliantPool.StaleCleanRoot.selector);
        withdraw(early, users[3], 0);
        // And against the current clean tree no proof exists at all.
        assertFalse(prove(1, 1, users[3], 0).ok, "the circuit refuses an excluded note");
        // A prover that pretends nothing was excluded gets a root the pool no longer accepts.
        uint256[] memory none = new uint256[](0);
        Proof memory pretend = proveWith(1, 1, none, users[3], 0);
        assertTrue(pretend.ok);
        vm.expectRevert(CompliantPool.StaleCleanRoot.selector);
        withdraw(pretend, users[3], 0);
    }

    function test_the_other_notes_still_withdraw_after_an_exclusion() public {
        depositAs(0, 0);
        depositAs(1, 1);
        depositAs(2, 2);
        blockAndExclude(1, 1);
        Proof memory p = prove(2, 2, users[3], 0);
        assertTrue(p.ok);
        assertEq(p.cleanRoot, pool.currentCleanRoot(), "SDK mask matches the on-chain mask");
        assertTrue(p.cleanRoot != p.poolRoot);
        withdraw(p, users[3], 0);
        assertEq(token.balanceOf(users[3]), 1_000e6 + DENOMINATION);
    }

    function test_nobody_can_exclude_an_authorized_depositor() public {
        depositAs(0, 0);
        vm.prank(makeAddr("griefer"));
        vm.expectRevert(CompliantPool.StillAuthorized.selector);
        pool.exclude(0);
    }

    function test_anyone_can_exclude_once_the_issuer_blocks() public {
        depositAs(0, 0);
        REGISTRY.modifyPolicyBlacklist(policy, users[0], true);
        vm.prank(makeAddr("keeper"));
        pool.exclude(0);
        vm.expectRevert(CompliantPool.AlreadyExcluded.selector);
        pool.exclude(0);
        vm.expectRevert(CompliantPool.UnknownIndex.selector);
        pool.exclude(1);
    }

    function test_unblocking_allows_reinclusion_and_the_note_withdraws_again() public {
        depositAs(0, 0);
        depositAs(1, 1);
        blockAndExclude(0, 0);
        vm.expectRevert(CompliantPool.StillUnauthorized.selector);
        pool.reinclude(0);

        REGISTRY.modifyPolicyBlacklist(policy, users[0], false);
        vm.prank(makeAddr("keeper"));
        pool.reinclude(0);
        excluded.pop();
        vm.expectRevert(CompliantPool.NotExcluded.selector);
        pool.reinclude(0);

        Proof memory p = prove(0, 0, users[3], 0);
        assertTrue(p.ok);
        assertEq(p.cleanRoot, pool.currentCleanRoot());
        withdraw(p, users[3], 0);
    }

    function test_a_blocked_recipient_cannot_be_paid() public {
        depositAs(0, 0);
        depositAs(1, 1);
        address blocked = makeAddr("blocked");
        REGISTRY.modifyPolicyBlacklist(policy, blocked, true);
        Proof memory p = prove(0, 0, blocked, 0);
        assertTrue(p.ok);
        // The proof is valid: the token's policy is what refuses the payment.
        vm.expectRevert(ITIP20.PolicyForbids.selector);
        withdraw(p, blocked, 0);
        assertFalse(pool.spent(p.nullifierHash), "the failed payment reverted the spend");
    }

    function test_a_blocked_address_cannot_deposit() public {
        REGISTRY.modifyPolicyBlacklist(policy, users[0], true);
        uint256 c = commitment(0);
        vm.prank(users[0]);
        vm.expectRevert(ITIP20.PolicyForbids.selector);
        pool.deposit(c);
        assertEq(pool.nextIndex(), 0, "the failed transfer reverted the insert");
    }

    function test_request_checks() public {
        depositAs(0, 0);
        Proof memory p = prove(0, 0, users[3], 0);
        vm.expectRevert(CompliantPool.InvalidRecipient.selector);
        pool.withdraw(p.a, p.b, p.c, p.poolRoot, p.cleanRoot, p.nullifierHash, address(0), relayer, 0);
        vm.expectRevert(CompliantPool.FeeTooHigh.selector);
        pool.withdraw(p.a, p.b, p.c, p.poolRoot, p.cleanRoot, p.nullifierHash, users[3], relayer, DENOMINATION + 1);
        vm.expectRevert(CompliantPool.RelayerRequired.selector);
        pool.withdraw(p.a, p.b, p.c, p.poolRoot, p.cleanRoot, p.nullifierHash, users[3], address(0), 1);
        vm.expectRevert(CompliantPool.UnknownPoolRoot.selector);
        pool.withdraw(p.a, p.b, p.c, 12345, p.cleanRoot, p.nullifierHash, users[3], relayer, 0);
        vm.expectRevert(CompliantPool.InvalidCommitment.selector);
        pool.deposit(0);
        vm.expectRevert(CompliantPool.InvalidCommitment.selector);
        pool.deposit(FIELD);
    }
}
