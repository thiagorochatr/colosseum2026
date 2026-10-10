// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {Groth16Verifier} from "../src/Groth16Verifier.sol";

/// The withdraw proof verified by the snarkjs-generated Solidity verifier. The
/// fixtures are the same proofs the Solana program verifies, converted by
/// snarkjs's own calldata exporter.
contract VerifierTest is Test {
    uint256 constant P = 21888242871839275222246405745257275088548364400416034343698204186575808495617;

    Groth16Verifier verifier;
    string json;

    struct Proof {
        uint256[2] a;
        uint256[2][2] b;
        uint256[2] c;
        uint256[4] inputs;
    }

    function setUp() public {
        verifier = new Groth16Verifier();
        json = vm.readFile("test/fixtures/proofs.json");
    }

    function load(uint256 i) internal view returns (Proof memory p) {
        string memory k = string.concat(".proofs[", vm.toString(i), "]");
        uint256[] memory a = vm.parseJsonUintArray(json, string.concat(k, ".a"));
        uint256[] memory b0 = vm.parseJsonUintArray(json, string.concat(k, ".b[0]"));
        uint256[] memory b1 = vm.parseJsonUintArray(json, string.concat(k, ".b[1]"));
        uint256[] memory c = vm.parseJsonUintArray(json, string.concat(k, ".c"));
        uint256[] memory s = vm.parseJsonUintArray(json, string.concat(k, ".inputs"));
        p.a = [a[0], a[1]];
        p.b = [[b0[0], b0[1]], [b1[0], b1[1]]];
        p.c = [c[0], c[1]];
        p.inputs = [s[0], s[1], s[2], s[3]];
    }

    function check(Proof memory p) internal view returns (bool) {
        return verifier.verifyProof(p.a, p.b, p.c, p.inputs);
    }

    function test_every_valid_proof_verifies() public view {
        uint256 worst;
        for (uint256 i = 0; i < 4; i++) {
            Proof memory p = load(i);
            uint256 before = gasleft();
            bool ok = check(p);
            uint256 used = before - gasleft();
            assertTrue(ok, "a valid proof must verify");
            if (used > worst) worst = used;
        }
        console.log("verifyProof, 4 public inputs, worst-case gas:", worst);
    }

    function test_changing_any_public_input_fails() public view {
        for (uint256 i = 0; i < 4; i++) {
            Proof memory p = load(0);
            p.inputs[i] = (p.inputs[i] + 1) % P;
            assertFalse(check(p));
        }
    }

    function test_the_input_order_is_part_of_the_statement() public view {
        Proof memory p = load(0);
        (p.inputs[0], p.inputs[1]) = (p.inputs[1], p.inputs[0]);
        assertFalse(check(p));
    }

    function test_one_notes_proof_does_not_verify_another_notes_inputs() public view {
        Proof memory p = load(0);
        p.inputs = load(1).inputs;
        assertFalse(check(p));
    }

    function test_swapped_g2_coordinates_fail() public view {
        Proof memory p = load(0);
        (p.b[0][0], p.b[0][1]) = (p.b[0][1], p.b[0][0]);
        assertFalse(check(p));
    }

    function test_a_non_canonical_input_is_refused() public view {
        Proof memory p = load(0);
        p.inputs[3] = P;
        assertFalse(check(p));
    }
}
