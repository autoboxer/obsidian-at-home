import type { ObjectDirective } from 'vue';

const activeBackdrops: HTMLElement[] = [];
const activeBackdropAttribute = 'data-modal-scroll-active';
const scrollableOverflow = new Set([ 'auto', 'overlay', 'scroll' ]);
const scrollingKeys = new Set([
  ' ',
  'ArrowDown',
  'ArrowUp',
  'End',
  'Home',
  'PageDown',
  'PageUp'
]);

interface TouchPosition {
  identifier: number;
  target: EventTarget | null;
  x: number;
  y: number;
}

let listenersAttached = false;
let touchPosition: TouchPosition | null = null;

function currentBackdrop(): HTMLElement | undefined {
  for ( let index = activeBackdrops.length - 1; index >= 0; index -= 1 ) {
    const backdrop = activeBackdrops[ index ];
    if ( backdrop?.isConnected ) {
      return backdrop;
    }
  }

  return undefined;
}

function modalSurface( backdrop: HTMLElement, target: EventTarget | null ): HTMLElement | undefined {
  if ( !( target instanceof Element ) || !backdrop.contains( target ) ) {
    return undefined;
  }
  const surface = target.closest<HTMLElement>( "[role='dialog']" );

  return surface && backdrop.contains( surface ) ? surface : undefined;
}

function scrollableOnAxis( element: HTMLElement, axis: 'x' | 'y' ): boolean {
  const style = window.getComputedStyle( element );
  if ( axis === 'x' ) {
    return scrollableOverflow.has( style.overflowX )
      && element.scrollWidth > element.clientWidth + 1;
  }

  return scrollableOverflow.has( style.overflowY )
    && element.scrollHeight > element.clientHeight + 1;
}

function canScroll( element: HTMLElement, axis: 'x' | 'y', delta: number ): boolean {
  if ( !scrollableOnAxis( element, axis ) || delta === 0 ) {
    return false;
  }
  if ( axis === 'x' ) {
    return delta < 0
      ? element.scrollLeft > 0
      : element.scrollLeft + element.clientWidth < element.scrollWidth - 1;
  }

  return delta < 0
    ? element.scrollTop > 0
    : element.scrollTop + element.clientHeight < element.scrollHeight - 1;
}

function scrollCandidates(
  backdrop: HTMLElement,
  target: EventTarget | null
): HTMLElement[] {
  const surface = modalSurface( backdrop, target );
  if ( !surface ) {
    return [];
  }
  const candidates: HTMLElement[] = [];
  let current = target instanceof Element ? target : null;
  while ( current && current !== backdrop ) {
    if ( current instanceof HTMLElement ) {
      candidates.push( current );
    }
    current = current.parentElement;
  }
  for ( const region of surface.querySelectorAll<HTMLElement>( '[data-modal-scroll-region]' ) ) {
    if ( !candidates.includes( region ) ) {
      candidates.push( region );
    }
  }

  return candidates;
}

function scrollTarget(
  backdrop: HTMLElement,
  target: EventTarget | null,
  axis: 'x' | 'y',
  delta: number
): HTMLElement | undefined {
  return scrollCandidates( backdrop, target ).find( ( candidate ) =>
    canScroll( candidate, axis, delta )
  );
}

function anyScrollTarget(
  backdrop: HTMLElement,
  target: EventTarget | null,
  axis: 'x' | 'y'
): HTMLElement | undefined {
  return scrollCandidates( backdrop, target ).find( ( candidate ) =>
    scrollableOnAxis( candidate, axis )
  );
}

function scrollFromInput(
  backdrop: HTMLElement,
  target: EventTarget | null,
  deltaX: number,
  deltaY: number
): void {
  const horizontal = scrollTarget( backdrop, target, 'x', deltaX );
  const vertical = scrollTarget( backdrop, target, 'y', deltaY );
  if ( horizontal && horizontal === vertical ) {
    horizontal.scrollBy({ left: deltaX, top: deltaY });

    return;
  }
  horizontal?.scrollBy({ left: deltaX });
  vertical?.scrollBy({ top: deltaY });
}

function normalizedWheelDelta( event: WheelEvent ): { x: number; y: number } {
  const scale = event.deltaMode === WheelEvent.DOM_DELTA_LINE
    ? 16
    : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
      ? Math.max( 1, currentBackdrop()?.clientHeight ?? window.innerHeight )
      : 1;
  let x = event.deltaX * scale;
  let y = event.deltaY * scale;
  if ( event.shiftKey && x === 0 ) {
    x = y;
    y = 0;
  }

  return { x, y };
}

function handleWheel( event: WheelEvent ): void {
  const backdrop = currentBackdrop();
  if ( !backdrop || event.ctrlKey ) {
    return;
  }
  event.preventDefault();
  const delta = normalizedWheelDelta( event );
  scrollFromInput( backdrop, event.target, delta.x, delta.y );
}

function handleTouchStart( event: TouchEvent ): void {
  if ( !currentBackdrop() || event.touches.length !== 1 ) {
    touchPosition = null;

    return;
  }
  const touch = event.touches[ 0 ];
  touchPosition = touch
    ? {
      identifier: touch.identifier,
      target: event.target,
      x: touch.clientX,
      y: touch.clientY
    }
    : null;
}

function handleTouchMove( event: TouchEvent ): void {
  const backdrop = currentBackdrop();
  if ( !backdrop ) {
    return;
  }
  if ( event.touches.length !== 1 ) {
    touchPosition = null;

    return;
  }
  if ( !touchPosition ) {
    return;
  }
  event.preventDefault();
  const touch = Array.from( event.touches ).find( ( candidate ) =>
    candidate.identifier === touchPosition?.identifier
  );
  if ( !touch ) {
    touchPosition = null;

    return;
  }
  const deltaX = touchPosition.x - touch.clientX;
  const deltaY = touchPosition.y - touch.clientY;
  scrollFromInput( backdrop, touchPosition.target, deltaX, deltaY );
  touchPosition.x = touch.clientX;
  touchPosition.y = touch.clientY;
}

function clearTouchPosition(): void {
  touchPosition = null;
}

function editingTarget( target: EventTarget | null ): HTMLElement | undefined {
  if ( !( target instanceof Element ) ) {
    return undefined;
  }

  return target.closest<HTMLElement>(
    "input, select, textarea, [contenteditable]:not([contenteditable='false'])"
  )
    ?? undefined;
}

function spaceActivatesControl( target: EventTarget | null ): boolean {
  return target instanceof Element
    && Boolean( target.closest( "button, input, label, select, summary, textarea, [role='button']" ) );
}

function handleKeydown( event: KeyboardEvent ): void {
  const backdrop = currentBackdrop();
  if (
    !backdrop
    || !scrollingKeys.has( event.key )
    || event.altKey
    || event.ctrlKey
    || event.metaKey
    || event.isComposing
  ) {
    return;
  }
  if ( editingTarget( event.target ) ) {
    return;
  }
  if ( event.key === ' ' && spaceActivatesControl( event.target ) ) {
    return;
  }
  event.preventDefault();
  if ( !modalSurface( backdrop, event.target ) ) {
    return;
  }

  if ( event.key === 'Home' || event.key === 'End' ) {
    const target = anyScrollTarget( backdrop, event.target, 'y' );
    target?.scrollTo({ top: event.key === 'Home' ? 0 : target.scrollHeight });

    return;
  }
  const direction = event.key === 'ArrowUp'
    || event.key === 'PageUp'
    || event.key === ' ' && event.shiftKey
    ? -1
    : 1;
  const target = scrollTarget( backdrop, event.target, 'y', direction );
  if ( !target ) {
    return;
  }
  const distance = event.key === 'ArrowDown' || event.key === 'ArrowUp'
    ? 40
    : Math.max( 40, target.clientHeight * 0.85 );
  target.scrollBy({ top: direction * distance });
}

function attachListeners(): void {
  if ( listenersAttached ) {
    return;
  }
  listenersAttached = true;
  document.addEventListener( 'wheel', handleWheel, { capture: true, passive: false });
  document.addEventListener( 'touchstart', handleTouchStart, { capture: true, passive: true });
  document.addEventListener( 'touchmove', handleTouchMove, { capture: true, passive: false });
  document.addEventListener( 'touchend', clearTouchPosition, { capture: true, passive: true });
  document.addEventListener( 'touchcancel', clearTouchPosition, { capture: true, passive: true });
  document.addEventListener( 'keydown', handleKeydown, true );
}

function detachListeners(): void {
  if ( !listenersAttached ) {
    return;
  }
  listenersAttached = false;
  document.removeEventListener( 'wheel', handleWheel, true );
  document.removeEventListener( 'touchstart', handleTouchStart, true );
  document.removeEventListener( 'touchmove', handleTouchMove, true );
  document.removeEventListener( 'touchend', clearTouchPosition, true );
  document.removeEventListener( 'touchcancel', clearTouchPosition, true );
  document.removeEventListener( 'keydown', handleKeydown, true );
  clearTouchPosition();
}

function lockBackdrop( backdrop: HTMLElement ): void {
  if ( activeBackdrops.includes( backdrop ) ) {
    return;
  }
  currentBackdrop()?.removeAttribute( activeBackdropAttribute );
  activeBackdrops.push( backdrop );
  backdrop.setAttribute( activeBackdropAttribute, '' );
  document.documentElement.setAttribute( 'data-modal-scroll-lock', '' );
  attachListeners();
}

function unlockBackdrop( backdrop: HTMLElement ): void {
  const index = activeBackdrops.indexOf( backdrop );
  if ( index >= 0 ) {
    activeBackdrops.splice( index, 1 );
  }
  backdrop.removeAttribute( activeBackdropAttribute );
  const activeBackdrop = currentBackdrop();
  if ( activeBackdrop ) {
    activeBackdrop.setAttribute( activeBackdropAttribute, '' );

    return;
  }
  document.documentElement.removeAttribute( 'data-modal-scroll-lock' );
  detachListeners();
}

export const modalScrollLock: ObjectDirective<HTMLElement> = {
  mounted: lockBackdrop,
  unmounted: unlockBackdrop
};
