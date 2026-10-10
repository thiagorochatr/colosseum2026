// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Script} from "forge-std/Script.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {CompliantPool, IWithdrawVerifier} from "../src/CompliantPool.sol";
import {Groth16Verifier} from "../src/Groth16Verifier.sol";

/// Deploys the verifier and a pool for an existing TIP-20 token, and records
/// the addresses in deployments/<chainId>.json. The Poseidon library is
/// deployed and linked by forge.
///
///   POOL_TOKEN=0x... forge script script/Deploy.s.sol --rpc-url <rpc> --account deployer --broadcast
contract Deploy is Script {
    function run() external {
        address token = vm.envAddress("POOL_TOKEN");
        uint256 denomination = vm.envOr("POOL_DENOMINATION", uint256(100e6));

        vm.startBroadcast();
        Groth16Verifier verifier = new Groth16Verifier();
        CompliantPool pool = new CompliantPool(ITIP20(token), denomination, IWithdrawVerifier(address(verifier)));
        vm.stopBroadcast();

        string memory o = "deployment";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeUint(o, "deployBlock", block.number);
        vm.serializeAddress(o, "token", token);
        vm.serializeUint(o, "denomination", denomination);
        vm.serializeAddress(o, "verifier", address(verifier));
        string memory json = vm.serializeAddress(o, "pool", address(pool));
        vm.writeJson(json, string.concat("deployments/", vm.toString(block.chainid), ".json"));
    }
}
