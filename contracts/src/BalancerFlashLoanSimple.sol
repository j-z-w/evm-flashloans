// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IERC20Minimal {
    function balanceOf(address account) external view returns (uint256);

    function transfer(address to, uint256 amount) external returns (bool);
}

interface IFlashLoanRecipient {
    function receiveFlashLoan(
        IERC20Minimal[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory feeAmounts,
        bytes memory userData
    ) external;
}

interface IBalancerVault {
    function flashLoan(
        IFlashLoanRecipient recipient,
        IERC20Minimal[] memory tokens,
        uint256[] memory amounts,
        bytes memory userData
    ) external;
}

contract BalancerFlashLoanSimple is IFlashLoanRecipient {
    error OnlyOwner();
    error OnlyOperator();
    error NotVault();
    error Paused();
    error Reentrancy();
    error FlashLoanInFlight();
    error InvalidLoanArrayLengths();
    error ZeroAmount();
    error TokenNotAllowed();
    error AmountExceedsMax();
    error FeeTooHigh();
    error UnexpectedCallbackToken();
    error UnexpectedCallbackAmount();
    error UnexpectedCallbackData();
    error InsufficientRepaymentBalance();
    error RepayTransferFailed();
    error ZeroAddressVault();
    error InvalidOwner();
    error InvalidOperator();
    error InvalidFeeBps();
    error InvalidWithdrawTo();
    error TokenBalanceQueryFailed();
    error IncompleteRepayment();
    error TokenNotContract();

    IBalancerVault public immutable vault;
    address public immutable owner;
    address public operator;
    bool public paused;

    struct TokenRiskConfig {
        bool enabled;
        uint256 maxLoanAmount;
        uint16 maxFeeBps;
    }

    mapping(address => TokenRiskConfig) public tokenRiskConfig;
    bool private _entered;
    bool private _inFlight;
    address private _expectedToken;
    uint256 private _expectedAmount;
    bytes32 private _expectedUserDataHash;

    bool public receivedFlashLoan;
    IERC20Minimal public lastToken;
    uint256 public lastAmount;
    uint256 public lastFeeAmount;
    bytes public lastUserData;

    event TokenRiskConfigUpdated(address indexed token, bool enabled, uint256 maxLoanAmount, uint16 maxFeeBps);
    event PauseStateSet(bool paused);
    event OperatorUpdated(address indexed previousOperator, address indexed newOperator);
    event FlashLoanRequested(address indexed token, uint256 amount, bytes32 indexed userDataHash);
    event FlashLoanRepaid(address indexed token, uint256 amount, uint256 feeAmount, bytes32 indexed userDataHash);
    event Withdrawal(address indexed token, address indexed to, uint256 amount);

    constructor(address vault_, address owner_, address operator_) {
        if (vault_ == address(0)) revert ZeroAddressVault();
        if (owner_ == address(0)) revert InvalidOwner();
        if (operator_ == address(0)) revert InvalidOperator();
        vault = IBalancerVault(vault_);
        owner = owner_;
        operator = operator_;
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert Paused();
        _;
    }

    modifier onlyOperator() {
        if (msg.sender != operator) revert OnlyOperator();
        _;
    }

    modifier nonReentrant() {
        if (_entered) revert Reentrancy();
        _entered = true;
        _;
        _entered = false;
    }

    function setPaused(bool paused_) external onlyOwner {
        paused = paused_;
        emit PauseStateSet(paused_);
    }

    function setOperator(address newOperator) external onlyOwner {
        if (newOperator == address(0)) revert InvalidOperator();
        address previousOperator = operator;
        operator = newOperator;
        emit OperatorUpdated(previousOperator, newOperator);
    }

    function setTokenRiskConfig(address token, bool enabled, uint256 maxLoanAmount, uint16 maxFeeBps) external onlyOwner {
        if (maxFeeBps > 10_000) revert InvalidFeeBps();
        _requireContractToken(token);
        tokenRiskConfig[token] = TokenRiskConfig({enabled: enabled, maxLoanAmount: maxLoanAmount, maxFeeBps: maxFeeBps});
        emit TokenRiskConfigUpdated(token, enabled, maxLoanAmount, maxFeeBps);
    }

    function executeFlashLoan(IERC20Minimal token, uint256 amount, bytes calldata userData)
        external
        onlyOperator
        whenNotPaused
        nonReentrant
    {
        if (_inFlight) revert FlashLoanInFlight();
        if (amount == 0) revert ZeroAmount();

        TokenRiskConfig memory risk = tokenRiskConfig[address(token)];
        if (!risk.enabled) revert TokenNotAllowed();
        if (amount > risk.maxLoanAmount) revert AmountExceedsMax();

        _inFlight = true;
        _expectedToken = address(token);
        _expectedAmount = amount;
        _expectedUserDataHash = keccak256(userData);

        emit FlashLoanRequested(address(token), amount, _expectedUserDataHash);

        IERC20Minimal[] memory tokens = new IERC20Minimal[](1);
        tokens[0] = token;

        uint256[] memory amounts = new uint256[](1);
        amounts[0] = amount;

        vault.flashLoan(this, tokens, amounts, userData);

        _inFlight = false;
        _expectedToken = address(0);
        _expectedAmount = 0;
        _expectedUserDataHash = bytes32(0);
    }

    function receiveFlashLoan(
        IERC20Minimal[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory feeAmounts,
        bytes memory userData
    ) external override whenNotPaused {
        if (msg.sender != address(vault)) revert NotVault();
        if (tokens.length != 1 || amounts.length != 1 || feeAmounts.length != 1) {
            revert InvalidLoanArrayLengths();
        }
        if (!_inFlight) revert FlashLoanInFlight();
        if (address(tokens[0]) != _expectedToken) revert UnexpectedCallbackToken();
        if (amounts[0] != _expectedAmount) revert UnexpectedCallbackAmount();
        if (keccak256(userData) != _expectedUserDataHash) revert UnexpectedCallbackData();

        TokenRiskConfig memory risk = tokenRiskConfig[address(tokens[0])];
        if (!risk.enabled) revert TokenNotAllowed();
        if (amounts[0] > risk.maxLoanAmount) revert AmountExceedsMax();

        uint256 vaultBalanceBefore = _balanceOf(address(tokens[0]), address(vault));
        uint256 maxFee = (amounts[0] * risk.maxFeeBps) / 10_000;
        if (feeAmounts[0] > maxFee) revert FeeTooHigh();

        uint256 repaymentAmount = amounts[0] + feeAmounts[0];
        if (_balanceOf(address(tokens[0]), address(this)) < repaymentAmount) revert InsufficientRepaymentBalance();
        _safeTransfer(address(tokens[0]), address(vault), repaymentAmount);
        if (_balanceOf(address(tokens[0]), address(vault)) < vaultBalanceBefore + repaymentAmount) {
            revert IncompleteRepayment();
        }

        receivedFlashLoan = true;
        lastToken = tokens[0];
        lastAmount = amounts[0];
        lastFeeAmount = feeAmounts[0];
        lastUserData = userData;

        emit FlashLoanRepaid(address(tokens[0]), amounts[0], feeAmounts[0], keccak256(userData));
    }

    function withdraw(IERC20Minimal token, address to, uint256 amount) external onlyOwner nonReentrant {
        if (to == address(0)) revert InvalidWithdrawTo();
        _safeTransfer(address(token), to, amount);
        emit Withdrawal(address(token), to, amount);
    }

    function inFlight() external view returns (bool) {
        return _inFlight;
    }

    function _safeTransfer(address token, address to, uint256 amount) internal {
        _requireContractToken(token);
        (bool success, bytes memory returnData) =
            token.call(abi.encodeWithSelector(IERC20Minimal.transfer.selector, to, amount));
        if (!success) revert RepayTransferFailed();
        if (returnData.length != 0) {
            if (returnData.length < 32) revert RepayTransferFailed();
            if (!abi.decode(returnData, (bool))) revert RepayTransferFailed();
        }
    }

    function _balanceOf(address token, address account) internal view returns (uint256 balance) {
        _requireContractToken(token);
        (bool success, bytes memory returnData) =
            token.staticcall(abi.encodeWithSelector(IERC20Minimal.balanceOf.selector, account));
        if (!success || returnData.length < 32) revert TokenBalanceQueryFailed();
        balance = abi.decode(returnData, (uint256));
    }

    function _requireContractToken(address token) internal view {
        if (token.code.length == 0) revert TokenNotContract();
    }
}
