// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {PoseidonT2} from "poseidon-solidity/PoseidonT2.sol";
import {PoseidonT3} from "poseidon-solidity/PoseidonT3.sol";
import {PoseidonT4} from "poseidon-solidity/PoseidonT4.sol";

/// poseidon-solidity agrees with circomlib and with mirror-core: the published
/// vector, the zero ladder, and the pool and clean roots in the circuit's test
/// vectors, rebuilt in Solidity from each note's secrets and Merkle path.
contract PoseidonTest is Test {
    uint256 constant DEPTH = 20;
    string vectors;

    function setUp() public {
        vectors = vm.readFile("../circuits/test/vectors.json");
    }

    function test_matches_the_published_circomlib_vector() public view {
        assertEq(PoseidonT3.hash([uint256(1), uint256(2)]), 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a);
    }

    function test_zero_ladder_matches_mirror_core() public view {
        uint256 z = 0;
        for (uint256 i = 0; i < DEPTH; i++) {
            z = PoseidonT3.hash([z, z]);
            if (i == 0) assertEq(z, 0x2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864);
        }
        assertEq(z, 0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3e);
    }

    /// Folds `leaf` up a path the way mirror-core's MerkleProof::fold does.
    function fold(uint256 leaf, uint256 index, uint256[] memory siblings) internal view returns (uint256 node) {
        node = leaf;
        for (uint256 j = 0; j < DEPTH; j++) {
            node = (index >> j) & 1 == 0 ? PoseidonT3.hash([node, siblings[j]]) : PoseidonT3.hash([siblings[j], node]);
        }
    }

    function test_rebuilds_the_vector_roots_from_each_notes_secrets() public view {
        uint256 denom = vm.parseJsonUint(vectors, ".valid[0].denomTag");
        for (uint256 i = 0; i < 4; i++) {
            string memory v = string.concat(".valid[", vm.toString(i), "]");
            uint256 k = vm.parseJsonUint(vectors, string.concat(v, ".k"));
            uint256 r = vm.parseJsonUint(vectors, string.concat(v, ".r"));
            uint256 index = vm.parseJsonUint(vectors, string.concat(v, "._index"));
            uint256 leaf = PoseidonT4.hash([k, r, denom]);
            assertEq(PoseidonT2.hash([k]), vm.parseJsonUint(vectors, string.concat(v, ".nullifierHash")), "nullifier");
            uint256[] memory pool = vm.parseJsonUintArray(vectors, string.concat(v, ".poolSiblings"));
            uint256[] memory clean = vm.parseJsonUintArray(vectors, string.concat(v, ".cleanSiblings"));
            assertEq(fold(leaf, index, pool), vm.parseJsonUint(vectors, string.concat(v, ".poolRoot")), "pool root");
            assertEq(fold(leaf, index, clean), vm.parseJsonUint(vectors, string.concat(v, ".cleanRoot")), "clean root");
        }
    }

    function test_gas_for_one_path_of_twenty_hashes() public view {
        uint256 node = 1;
        uint256 before = gasleft();
        for (uint256 j = 0; j < DEPTH; j++) {
            node = PoseidonT3.hash([node, j]);
        }
        uint256 used = before - gasleft();
        console.log("20 x PoseidonT3, gas:", used);
        assertGt(node, 0);
    }
}
