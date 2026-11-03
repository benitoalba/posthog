from django.db.models import Prefetch

from ...models.review_queues import ReviewQueueItem
from ...models.trace_reviews import TraceReview, TraceReviewScore
from .review import HumanReview, QueueAssignment, ReviewScore, TraceReviewState, reviewer_name, score_display_value


def load_trace_review_state(team_id: int, trace_id: str) -> TraceReviewState:
    """The trace's active human review and review queue item. A trace has at most one of each."""
    return TraceReviewState(review=_load_review(team_id, trace_id), queue=_load_queue_assignment(team_id, trace_id))


def _load_review(team_id: int, trace_id: str) -> HumanReview | None:
    scores = TraceReviewScore.objects.select_related("definition").order_by("definition__name", "id")
    review = (
        TraceReview.objects.filter(team_id=team_id, trace_id=trace_id, deleted=False)
        .select_related("reviewed_by")
        .prefetch_related(Prefetch("scores", queryset=scores))
        .first()
    )
    if review is None:
        return None
    reviewer = review.reviewed_by
    return HumanReview(
        reviewed_by=reviewer_name(reviewer.first_name, reviewer.email, reviewer.distinct_id) if reviewer else None,
        reviewed_at=review.updated_at or review.created_at,
        comment=review.comment,
        scores=tuple(
            ReviewScore(
                name=score.definition.name,
                value=score_display_value(
                    score.definition.kind,
                    score.definition_config if isinstance(score.definition_config, dict) else None,
                    score.categorical_values,
                    score.numeric_value,
                    score.boolean_value,
                ),
            )
            for score in review.scores.all()
        ),
    )


def _load_queue_assignment(team_id: int, trace_id: str) -> QueueAssignment | None:
    item = (
        ReviewQueueItem.objects.filter(team_id=team_id, trace_id=trace_id, deleted=False, queue__deleted=False)
        .select_related("queue")
        .first()
    )
    if item is None:
        return None
    return QueueAssignment(queue_id=str(item.queue_id), queue_name=item.queue.name, item_id=str(item.id))
