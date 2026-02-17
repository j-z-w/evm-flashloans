// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {BalancerFlashLoanSimple, IERC20Minimal, IFlashLoanRecipient} from "../src/BalancerFlashLoanSimple.sol";

contract MockToken is IERC20Minimal {
    mapping(address => uint256) public balances;

    function mint(address to, uint256 amount) external {
        balances[to] += amount;
    }

    function balanceOf(address account) external view returns (uint256) {
        return balances[account];
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        if (balances[msg.sender] < amount) return false;
        balances[msg.sender] -= amount;
        balances[to] += amount;
        return true;
    }
}

contract MockTokenNoReturn {
    mapping(address => uint256) public balances;

    function mint(address to, uint256 amount) external {
        balances[to] += amount;
    }

    function balanceOf(address account) external view returns (uint256) {
        return balances[account];
    }

    function transfer(address to, uint256 amount) external {
        require(balances[msg.sender] >= amount, "insufficient");
        balances[msg.sender] -= amount;
        balances[to] += amount;
    }
}

contract MockFeeOnTransferToken is IERC20Minimal {
    mapping(address => uint256) public balances;
    uint16 public immutable transferFeeBps;

    constructor(uint16 transferFeeBps_) {
        transferFeeBps = transferFeeBps_;
    }

    function mint(address to, uint256 amount) external {
        balances[to] += amount;
    }

    function balanceOf(address account) external view returns (uint256) {
        return balances[account];
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        if (balances[msg.sender] < amount) return false;
        balances[msg.sender] -= amount;

        uint256 fee = (amount * transferFeeBps) / 10_000;
        uint256 received = amount - fee;
        balances[to] += received;
        return true;
    }
}

contract MockBalancerVault {
    uint16 public feeBps;

    function setFeeBps(uint16 feeBps_) external {
        feeBps = feeBps_;
    }

    function flashLoan(
        IFlashLoanRecipient recipient,
        IERC20Minimal[] memory tokens,
        uint256[] memory amounts,
        bytes memory userData
    ) external {
        require(tokens.length == 1, "token len");
        require(amounts.length == 1, "amount len");

        uint256[] memory feeAmounts = new uint256[](1);
        feeAmounts[0] = (amounts[0] * feeBps) / 10_000;

        _safeTransfer(address(tokens[0]), address(recipient), amounts[0]);
        recipient.receiveFlashLoan(tokens, amounts, feeAmounts, userData);
    }

    function _safeTransfer(address token, address to, uint256 amount) internal {
        (bool success, bytes memory returnData) =
            token.call(abi.encodeWithSelector(IERC20Minimal.transfer.selector, to, amount));
        require(success, "transfer out failed");
        if (returnData.length > 0) {
            require(returnData.length >= 32, "short return");
            require(abi.decode(returnData, (bool)), "transfer false");
        }
    }
}

contract BalancerFlashLoanSimpleGuardsTest is Test {
    MockToken internal token;
    MockBalancerVault internal vault;
    BalancerFlashLoanSimple internal receiver;

    function setUp() public {
        token = new MockToken();
        vault = new MockBalancerVault();
        receiver = new BalancerFlashLoanSimple(address(vault), address(this), address(this));

        token.mint(address(vault), 10_000 ether);
        token.mint(address(receiver), 50 ether);

        receiver.setTokenRiskConfig(address(token), true, 100 ether, 100); // 1% max fee
    }

    function testMockFlashLoanBorrowAndRepay() public {
        uint256 vaultBalanceBefore = token.balanceOf(address(vault));
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 10 ether, bytes("mock-ok"));
        uint256 vaultBalanceAfter = token.balanceOf(address(vault));

        assertTrue(receiver.receivedFlashLoan(), "callback not received");
        assertEq(vaultBalanceAfter, vaultBalanceBefore, "vault not repaid");
    }

    function testFeeCapBlocksRepayment() public {
        vault.setFeeBps(300); // 3% > 1% guard

        vm.expectRevert(BalancerFlashLoanSimple.FeeTooHigh.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 10 ether, bytes("high-fee"));
    }

    function testTokenWhitelistBlocksLoan() public {
        receiver.setTokenRiskConfig(address(token), false, 0, 100);

        vm.expectRevert(BalancerFlashLoanSimple.TokenNotAllowed.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 1 ether, bytes("blocked"));
    }

    function testAmountCapBlocksLoan() public {
        receiver.setTokenRiskConfig(address(token), true, 1 ether, 100);

        vm.expectRevert(BalancerFlashLoanSimple.AmountExceedsMax.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 2 ether, bytes("too-much"));
    }

    function testOnlyOwnerAdminAndWithdraw() public {
        vm.prank(address(0x1234));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setPaused(true);

        vm.prank(address(0x1234));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setTokenRiskConfig(address(token), true, 100 ether, 100);

        vm.prank(address(0x1234));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.setOperator(address(0x5678));

        vm.prank(address(0x1234));
        vm.expectRevert(BalancerFlashLoanSimple.OnlyOwner.selector);
        receiver.withdraw(IERC20Minimal(address(token)), address(0x1234), 1 ether);
    }

    function testOnlyOperatorCanExecute() public {
        address hotWallet = address(0xB0B);
        receiver.setOperator(hotWallet);

        vm.expectRevert(BalancerFlashLoanSimple.OnlyOperator.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 1 ether, bytes("old-operator"));

        vm.prank(hotWallet);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 1 ether, bytes("new-operator"));
    }

    function testSetOperatorRejectsZeroAddress() public {
        vm.expectRevert(BalancerFlashLoanSimple.InvalidOperator.selector);
        receiver.setOperator(address(0));
    }

    function testPauseBlocksBothEntryPoints() public {
        receiver.setPaused(true);

        vm.expectRevert(BalancerFlashLoanSimple.Paused.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(token)), 1 ether, bytes("paused"));

        IERC20Minimal[] memory tokens = new IERC20Minimal[](1);
        tokens[0] = IERC20Minimal(address(token));
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = 1 ether;
        uint256[] memory fees = new uint256[](1);
        fees[0] = 0;

        vm.prank(address(vault));
        vm.expectRevert(BalancerFlashLoanSimple.Paused.selector);
        receiver.receiveFlashLoan(tokens, amounts, fees, bytes("paused-callback"));
    }

    function testCallbackWithoutInFlightReverts() public {
        IERC20Minimal[] memory tokens = new IERC20Minimal[](1);
        tokens[0] = IERC20Minimal(address(token));
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = 1 ether;
        uint256[] memory fees = new uint256[](1);
        fees[0] = 0;

        vm.prank(address(vault));
        vm.expectRevert(BalancerFlashLoanSimple.FlashLoanInFlight.selector);
        receiver.receiveFlashLoan(tokens, amounts, fees, bytes("unexpected"));
    }

    function testNoReturnTokenTransferSupported() public {
        MockTokenNoReturn noReturnToken = new MockTokenNoReturn();
        noReturnToken.mint(address(vault), 500 ether);
        noReturnToken.mint(address(receiver), 5 ether);

        receiver.setTokenRiskConfig(address(noReturnToken), true, 10 ether, 500);
        receiver.executeFlashLoan(IERC20Minimal(address(noReturnToken)), 1 ether, bytes("no-return-ok"));

        assertTrue(receiver.receivedFlashLoan(), "no-return token callback not received");
        assertEq(address(receiver.lastToken()), address(noReturnToken), "unexpected token");
    }

    function testFeeOnTransferTokenFailsRepaymentInvariant() public {
        MockFeeOnTransferToken feeToken = new MockFeeOnTransferToken(1000); // 10% transfer fee
        feeToken.mint(address(vault), 500 ether);
        feeToken.mint(address(receiver), 50 ether);

        receiver.setTokenRiskConfig(address(feeToken), true, 10 ether, 10_000);

        vm.expectRevert(BalancerFlashLoanSimple.IncompleteRepayment.selector);
        receiver.executeFlashLoan(IERC20Minimal(address(feeToken)), 1 ether, bytes("fot-fail"));
    }

    function testRejectsNonContractTokenInConfig() public {
        vm.expectRevert(BalancerFlashLoanSimple.TokenNotContract.selector);
        receiver.setTokenRiskConfig(address(0x1234), true, 1 ether, 100);
    }

    function testWithdrawRejectsNonContractTokenAddress() public {
        vm.expectRevert(BalancerFlashLoanSimple.TokenNotContract.selector);
        receiver.withdraw(IERC20Minimal(address(0x5678)), address(this), 1);
    }
}
