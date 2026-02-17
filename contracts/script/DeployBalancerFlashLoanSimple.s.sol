// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {BalancerFlashLoanSimple} from "../src/BalancerFlashLoanSimple.sol";

contract DeployBalancerFlashLoanSimpleScript is Script {
    address internal constant DEFAULT_BALANCER_VAULT = 0xBA12222222228d8Ba445958a75a0704d566BF2C8;

    function run() external returns (BalancerFlashLoanSimple deployed) {
        address vault = vm.envOr("BALANCER_VAULT", DEFAULT_BALANCER_VAULT);
        address owner = vm.envAddress("BALANCER_OWNER");
        address operator = vm.envAddress("BALANCER_OPERATOR");

        vm.startBroadcast();
        deployed = new BalancerFlashLoanSimple(vault, owner, operator);
        vm.stopBroadcast();

        console2.log("BalancerFlashLoanSimple deployed:", address(deployed));
        console2.log("Vault:", vault);
        console2.log("Owner (multisig):", owner);
        console2.log("Operator (bot signer):", operator);
    }
}
