// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {BalancerFlashLoanSimple, IERC20Minimal} from "../src/BalancerFlashLoanSimple.sol";

contract BalancerFlashLoanSimpleTest is Test {
    address internal constant DEFAULT_BALANCER_VAULT = 0xBA12222222228d8Ba445958a75a0704d566BF2C8;
    address internal constant DEFAULT_BASE_WETH = 0x4200000000000000000000000000000000000006;

    BalancerFlashLoanSimple internal receiver;
    address internal vault;
    address internal loanToken;
    uint256 internal loanAmount;
    bool internal skipForkTests;

    modifier forkOnly() {
        vm.skip(skipForkTests, "BASE_RPC_HTTPS_URL not set; skipping fork integration test");
        _;
    }

    function setUp() public {
        string memory rpcUrl = vm.envOr("BASE_RPC_HTTPS_URL", string(""));
        bool requireForkTests = vm.envOr("REQUIRE_FORK_TESTS", false);
        string memory ci = vm.envOr("CI", string(""));
        if (bytes(ci).length != 0) {
            requireForkTests = true;
        }

        if (bytes(rpcUrl).length == 0) {
            if (requireForkTests) {
                revert("BASE_RPC_HTTPS_URL required when REQUIRE_FORK_TESTS=true or CI is set");
            }
            skipForkTests = true;
            return;
        }
        vm.createSelectFork(rpcUrl);

        vault = vm.envOr("BALANCER_VAULT", DEFAULT_BALANCER_VAULT);
        loanToken = vm.envOr("BALANCER_FLASH_TOKEN", DEFAULT_BASE_WETH);
        loanAmount = vm.envOr("BALANCER_FLASH_AMOUNT", uint256(1e15)); // 0.001 WETH default

        receiver = new BalancerFlashLoanSimple(vault, address(this), address(this));

        uint256 vaultTokenBalance = IERC20Minimal(loanToken).balanceOf(vault);
        assertGt(vaultTokenBalance, 0, "Vault has no balance for selected token");

        uint256 maxReasonableBorrow = vaultTokenBalance / 10_000;
        if (maxReasonableBorrow == 0) maxReasonableBorrow = 1;
        if (loanAmount > maxReasonableBorrow) {
            loanAmount = maxReasonableBorrow;
        }

        // Keep a buffer so the receiver can always pay any non-zero flash loan fee.
        deal(loanToken, address(receiver), loanAmount);

        receiver.setTokenRiskConfig(loanToken, true, loanAmount, 10_000);
    }

    function testFlashLoanBorrowAndRepay() public forkOnly {
        bytes memory userData = abi.encode("no-swap-simple-payback");
        uint256 vaultBalanceBefore = IERC20Minimal(loanToken).balanceOf(vault);

        receiver.executeFlashLoan(IERC20Minimal(loanToken), loanAmount, userData);

        uint256 vaultBalanceAfter = IERC20Minimal(loanToken).balanceOf(vault);

        assertTrue(receiver.receivedFlashLoan(), "flash callback not observed");
        assertEq(address(receiver.lastToken()), loanToken, "unexpected loan token");
        assertEq(receiver.lastAmount(), loanAmount, "unexpected loan amount");
        assertGe(vaultBalanceAfter, vaultBalanceBefore, "vault not repaid");
        assertEq(keccak256(receiver.lastUserData()), keccak256(userData), "unexpected userData");
        assertFalse(receiver.inFlight(), "flash loan state not reset");
    }

    function testPausedBlocksExecution() public forkOnly {
        receiver.setPaused(true);

        vm.expectRevert(BalancerFlashLoanSimple.Paused.selector);
        receiver.executeFlashLoan(IERC20Minimal(loanToken), loanAmount, bytes("paused"));
    }

    function testAmountCapBlocksExecution() public forkOnly {
        receiver.setTokenRiskConfig(loanToken, true, loanAmount - 1, 10_000);

        vm.expectRevert(BalancerFlashLoanSimple.AmountExceedsMax.selector);
        receiver.executeFlashLoan(IERC20Minimal(loanToken), loanAmount, bytes("too-much"));
    }

    function testOnlyOwnerAdminFunctions() public forkOnly {
        vm.prank(address(0xBEEF));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setPaused(true);

        vm.prank(address(0xBEEF));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setTokenRiskConfig(loanToken, true, loanAmount, 10_000);

        vm.prank(address(0xBEEF));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setOperator(address(0xCAFE));
    }

    function testOnlyOperatorCanExecute() public forkOnly {
        address hotWallet = address(0xCAFE);
        receiver.setOperator(hotWallet);

        vm.expectRevert(BalancerFlashLoanSimple.OnlyOperator.selector);
        receiver.executeFlashLoan(IERC20Minimal(loanToken), loanAmount, bytes("not-operator"));

        vm.prank(hotWallet);
        receiver.executeFlashLoan(IERC20Minimal(loanToken), loanAmount, bytes("operator-ok"));
    }
}
