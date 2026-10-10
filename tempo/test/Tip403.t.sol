// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP20RolesAuth} from "tempo-std/interfaces/ITIP20RolesAuth.sol";
import {ITIP403Registry} from "tempo-std/interfaces/ITIP403Registry.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";

/// The TIP-403 behaviour the pool relies on, against Tempo's real precompiles.
/// Run with `forge test --network tempo`.
contract Tip403Test is Test {
    ITIP403Registry constant REGISTRY = StdPrecompiles.TIP403_REGISTRY;
    ITIP20 constant PATH_USD = ITIP20(0x20C0000000000000000000000000000000000000);

    address alice = makeAddr("alice");
    address pool = makeAddr("pool");
    ITIP20 token;

    function setUp() public {
        token = ITIP20(
            StdPrecompiles.TIP20_FACTORY.createToken("Demo USD", "dUSD", "USD", PATH_USD, address(this), bytes32("demo"))
        );
        ITIP20RolesAuth(address(token)).grantRole(token.ISSUER_ROLE(), address(this));
        token.mint(alice, 1_000e6);
    }

    function blacklistToken() internal returns (uint64 id) {
        id = REGISTRY.createPolicy(address(this), ITIP403Registry.PolicyType.BLACKLIST);
        token.changeTransferPolicyId(id);
        assertEq(token.transferPolicyId(), id, "the token reports its policy");
    }

    function test_a_new_token_defaults_to_always_allow() public view {
        assertEq(token.transferPolicyId(), 1);
        assertEq(token.decimals(), 6);
    }

    function test_blacklisting_flips_both_checks_the_pool_can_read() public {
        uint64 id = blacklistToken();
        assertTrue(REGISTRY.isAuthorized(id, alice));
        assertTrue(REGISTRY.isAuthorizedSender(id, alice));
        REGISTRY.modifyPolicyBlacklist(id, alice, true);
        assertFalse(REGISTRY.isAuthorized(id, alice));
        assertFalse(REGISTRY.isAuthorizedSender(id, alice));
        REGISTRY.modifyPolicyBlacklist(id, alice, false);
        assertTrue(REGISTRY.isAuthorizedSender(id, alice));
    }

    function test_a_whitelist_authorizes_only_listed_addresses() public {
        uint64 id = REGISTRY.createPolicy(address(this), ITIP403Registry.PolicyType.WHITELIST);
        assertFalse(REGISTRY.isAuthorizedSender(id, alice));
        REGISTRY.modifyPolicyWhitelist(id, alice, true);
        assertTrue(REGISTRY.isAuthorizedSender(id, alice));
    }

    function test_the_token_itself_refuses_a_blacklisted_sender() public {
        uint64 id = blacklistToken();
        vm.prank(alice);
        token.approve(pool, 100e6);
        vm.prank(pool);
        assertTrue(token.transferFrom(alice, pool, 10e6));
        REGISTRY.modifyPolicyBlacklist(id, alice, true);
        vm.prank(pool);
        vm.expectRevert();
        token.transferFrom(alice, pool, 10e6);
    }

    function test_the_token_refuses_a_blacklisted_recipient() public {
        uint64 id = blacklistToken();
        vm.prank(alice);
        token.transfer(pool, 50e6);
        REGISTRY.modifyPolicyBlacklist(id, alice, true);
        vm.prank(pool);
        vm.expectRevert();
        token.transfer(alice, 1e6);
    }

    function test_a_compound_policy_answers_the_sender_question_from_its_sender_part() public {
        uint64 senders = REGISTRY.createPolicy(address(this), ITIP403Registry.PolicyType.BLACKLIST);
        uint64 id = REGISTRY.createCompoundPolicy(senders, 1, 1);
        token.changeTransferPolicyId(id);
        assertTrue(REGISTRY.isAuthorizedSender(id, alice));
        REGISTRY.modifyPolicyBlacklist(senders, alice, true);
        assertFalse(REGISTRY.isAuthorizedSender(id, alice));
        // The recipient side is still always-allow, so only the sender check moved.
        assertTrue(REGISTRY.isAuthorizedRecipient(id, alice));
        vm.prank(alice);
        vm.expectRevert();
        token.transfer(pool, 1e6);
    }
}
