import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zcash_wallet/src/core/layout/app_form_factor.dart';
import 'package:zcash_wallet/src/features/ledger/services/ledger_signed_operation_service.dart';
import 'package:zcash_wallet/src/features/payment_links/services/payment_link_recovery_store.dart';

import '../../support/ledger_gift_card_support.dart';
import '../../support/payment_links_screen_support.dart';

void main() {
  setUpAll(loadPaymentLinksTestFonts);

  for (final outcome in ['resume', 'cancel', 'missing']) {
    testWidgets('saved Ledger gift card recovery: $outcome', (tester) async {
      final h = LedgerGiftHarness();
      final draft = await h.service.prepare(
        accountUuid: 'account-1',
        amountZatoshi: ledgerGiftLink.amountZatoshi,
      );
      if (outcome != 'cancel') {
        await h.service.prove(accountUuid: 'account-1', draft: draft);
      }
      if (outcome == 'resume') {
        await h.operations.checkpoint(
          operationId: h.service.operationId('account-1', draft.link.address),
          accountUuid: 'account-1',
          kind: LedgerSignedOperationKind.giftCard,
          externalRef: draft.link.address,
          pcztWithProofsBytes: [2],
          pcztWithSignaturesBytes: [3],
        );
      }
      await pumpPaymentLinksScreen(
        tester,
        bootstrap: ledgerGiftBootstrap,
        ledgerFunding: h.service,
        operations: _RecoveryOperations(h.recovery),
        ledgerSigner: (_, _) => throw StateError('Must not sign again'),
        logicalSize: kAppFormFactor == AppFormFactor.mobile
            ? const Size(390, 844)
            : const Size(1080, 720),
      );
      await tester.pumpAndSettle();
      expect(find.text('Funding incomplete'), findsOneWidget);
      await tester.tap(
        find.text(outcome == 'cancel' ? 'Cancel draft' : 'Resume funding'),
      );
      await tester.pumpAndSettle();

      switch (outcome) {
        case 'resume':
          expect(h.operations.broadcasts, 1);
          expect(h.operations.checkpoints, 1);
          expect(h.operations.acks, 1);
          expect(
            (await h.recovery.load()).single.state,
            PaymentLinkRecoveryState.funded,
          );
          expect(find.text('Resume funding'), findsNothing);
        case 'cancel':
          expect(h.operations.broadcasts, 0);
          expect(await h.recovery.load(), isEmpty);
          expect(find.text('Cancel draft'), findsNothing);
        case 'missing':
          expect(h.operations.broadcasts, 0);
          expect(
            (await h.recovery.load()).single.state,
            PaymentLinkRecoveryState.draft,
          );
          expect(find.textContaining('Keep Vizor synced'), findsOneWidget);
      }
    });
  }
}

class _RecoveryOperations extends FakePaymentLinkOperations {
  _RecoveryOperations(this.store);
  final PaymentLinkRecoveryStore store;

  @override
  Future<List<PaymentLinkRecoveryRecord>> loadCreatedLinkRecoveries() =>
      store.load();
}
